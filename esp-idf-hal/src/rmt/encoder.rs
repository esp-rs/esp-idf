mod copy_encoder;
pub use copy_encoder::*;
mod bytes_encoder;
pub use bytes_encoder::*;

#[cfg(esp_idf_version_at_least_5_3_0)]
#[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_3_0)))]
pub mod simple_encoder;

use alloc::boxed::Box;
use core::mem;

use enumset::{EnumSet, EnumSetType};

use esp_idf_sys::*;

/// This trait represents an RMT encoder that is used to encode data for transmission.
pub trait RawEncoder {
    /// The type of input data the encoder can encode.
    type Item;

    /// Returns a mutable reference to the underlying `rmt_encoder_t`.
    ///
    /// The functions of the `rmt_encoder_t` will be called by the RMT driver.
    fn handle(&mut self) -> &mut rmt_encoder_t;

    /// Reset the encoder state.
    ///
    /// # Errors
    ///
    /// If resetting the encoder fails, it should return an [`EspError`]
    /// with the code [`ESP_FAIL`].
    fn reset(&mut self) -> Result<(), EspError> {
        esp!(unsafe { rmt_encoder_reset(self.handle()) })
    }
}

impl<E: RawEncoder> RawEncoder for &mut E {
    type Item = E::Item;

    fn handle(&mut self) -> &mut rmt_encoder_t {
        (*self).handle()
    }

    fn reset(&mut self) -> Result<(), EspError> {
        (*self).reset()
    }
}

/// RMT encoding state flags.
///
/// Mirrors `rmt_encode_state_t` (`rmt_encoder.h`), a C bitmask - a single result can combine
/// [`Complete`](Self::Complete) with [`MemFull`](Self::MemFull) and/or [`WithEof`](Self::WithEof)
/// at once. The reset state is the empty set (`EnumSet::empty()`).
///
/// Discriminants match the `RMT_ENCODING_*` raw values directly (`#[enumset(map = "mask")]`).
#[derive(Debug, EnumSetType)]
#[enumset(repr = "u32", map = "mask")]
pub enum EncoderState {
    /// The encoding session is finished, the caller can continue with subsequent encoding.
    Complete = 1,
    /// The encoding artifact memory is full, the caller should return from the current encoding session.
    MemFull = 2,
    /// The encoding session has inserted the EOF marker into the symbol stream.
    #[cfg(esp_idf_version_at_least_5_5_0)]
    #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_5_0)))]
    WithEof = 4,
}

/// A handle to an RMT channel.
///
/// For the special encoders like [`CopyEncoder`] or [`BytesEncoder`] that
/// directly write to the RMT memory, this handle is used to access the channel.
///
/// For third-party encoders this functionality is not exposed. To prevent misuse,
/// this type wraps the raw handle and does not expose it.
#[derive(Debug)]
pub struct RmtChannelHandle {
    handle: rmt_channel_handle_t,
}

impl RmtChannelHandle {
    /// Returns the underlying `rmt_channel_handle_t`.
    ///
    /// Note that this does not guarantee that the handle is still valid.
    #[must_use]
    pub(crate) fn as_ptr(&self) -> rmt_channel_handle_t {
        // This is explicitly not public to prevent misuse.
        //
        // Calling any of the ESP-IDF functions will likely violate invariants
        // of `TxChannelDriver` or `RxChannelDriver`.
        // The handle is only used by the encoders that copy the data directly
        // into the RMT memory (e.g. `CopyEncoder` or `BytesEncoder`).
        self.handle
    }
}

/// A trait for implementing custom RMT encoders in rust.
///
/// An RMT encoder is part of the RMT TX transaction, whose responsibility
/// is to generate and write the correct RMT symbols into hardware memory
/// or DMA buffer at a specific time.
///
/// There are some special restrictions for an encoding function:
/// - During a single transaction, the encoding function may be called
///   multiple times. This is necessary because the target RMT memory
///   block cannot hold all the artifacts at once. To overcome this
///   limitation, the driver utilizes a ping-pong approach, where the
///   encoding session is divided into multiple parts. This means that
///   the encoder needs to keep track of its state to continue encoding
///   from where it left off in the previous part.
/// - The encoding function is running in the ISR context.
///   To speed up the encoding session, it is highly recommended to put
///   the encoding function into IRAM. This can also avoid the cache miss during encoding.
pub trait Encoder {
    /// The type of input data the encoder can encode.
    type Item;

    /// Encode the user data into RMT symbols and write into RMT memory.
    ///
    /// This function might be called multiple times within a single transaction.
    /// The encode function should return the state of the current encoding session.
    ///
    /// The supported states are listed in [`EncoderState`]. If the result contains
    /// [`EncoderState::Complete`], the current encoder has finished work (this may be combined
    /// with either of the flags below in the same result - see [`EncoderState`]'s own doc
    /// comment).
    ///
    /// If the result contains [`EncoderState::MemFull`], the program needs to yield from the
    /// current session, as there is no space to save more encoding artifacts.
    ///
    /// # Note
    ///
    /// It is recommended to put this function implementation in the IRAM, to achieve a high
    /// performance and less interrupt latency.
    ///
    /// # ISR Safety
    ///
    /// The encoding function will also be called from an ISR context, thus the function must not
    /// call any blocking API.
    fn encode(
        &mut self,
        handle: &mut RmtChannelHandle,
        primary_data: &[Self::Item],
    ) -> (usize, EnumSet<EncoderState>);

    /// Reset encoding state.
    ///
    /// # Errors
    ///
    /// With `ESP_FAIL` when it fails to reset the encoder.
    fn reset(&mut self) -> Result<(), EspError>;
}

impl<E: RawEncoder> Encoder for E {
    type Item = E::Item;

    fn encode(
        &mut self,
        handle: &mut RmtChannelHandle,
        primary_data: &[Self::Item],
    ) -> (usize, EnumSet<EncoderState>) {
        let encoder_handle = self.handle();
        let Some(encode) = encoder_handle.encode else {
            return (0, EnumSet::empty());
        };

        let mut ret_state: rmt_encode_state_t = rmt_encode_state_t_RMT_ENCODING_RESET;
        let data_size = mem::size_of_val::<[Self::Item]>(primary_data);

        let written = unsafe {
            encode(
                encoder_handle,
                handle.as_ptr(),
                primary_data.as_ptr() as *const core::ffi::c_void,
                data_size,
                &raw mut ret_state,
            )
        };

        (written, EnumSet::from_repr_truncated(ret_state))
    }

    fn reset(&mut self) -> Result<(), EspError> {
        let handle = self.handle();
        if let Some(reset) = handle.reset {
            esp!(unsafe { reset(handle) })?;
        }

        Ok(())
    }
}

/// A helper function to convert a custom RMT encoder into a raw encoder
/// that can be used by the RMT driver.
///
/// # Panics
///
/// If the allocation for the encoder wrapper fails.
#[must_use]
pub fn into_raw<E: Encoder>(encoder: E) -> EncoderWrapper<E> {
    EncoderWrapper::new(encoder).expect("Failed to allocate memory for RMT encoder")
}

#[derive(Debug)]
struct InternalEncoderWrapper<E> {
    base: rmt_encoder_t, // the base "class", declares the standard encoder interface
    encoder: Option<E>,
}

/// A type implementing [`Encoder`] can not be directly used by the driver,
/// because the driver expects a different interface (see [`RawEncoder`]).
///
/// This type adapts an [`Encoder`] to a [`RawEncoder`], taking care of the
/// unsafe parts.
#[derive(Debug)]
#[repr(C)]
pub struct EncoderWrapper<E>(Box<InternalEncoderWrapper<E>>);

impl<E: Encoder> EncoderWrapper<E> {
    pub fn new(encoder: E) -> Result<Self, EspError> {
        Ok(Self(Box::new(InternalEncoderWrapper::new(encoder))))
    }
}

impl<E: Encoder> InternalEncoderWrapper<E> {
    /// This function returns a pointer to self from a pointer to its field `base`.
    ///
    /// One might assume that this is redundant, because the `base` field is the first field in the struct
    /// -> the pointer would point to the same address as the struct itself.
    /// but this is not guaranteed by rust.
    ///
    /// In the C representation, it might add padding before the fields to align them.
    /// In the rust representation, it doesn't even guarantee that the field is at
    /// the start of the struct.
    ///
    /// See <https://doc.rust-lang.org/reference/type-layout.html>
    unsafe fn containerof(ptr: *mut rmt_encoder_t) -> *mut Self {
        // The given pointer points to the base field of this struct,
        // in the C-Code they use the __containerof macro to get the pointer to the whole struct
        //
        // The below does the same.
        //
        // SAFETY: This struct is #[repr(C)]
        ptr.cast::<u8>()
            .sub(mem::offset_of!(Self, base))
            .cast::<Self>()
    }

    pub fn new(encoder: E) -> Self {
        Self {
            base: rmt_encoder_t {
                encode: Some(Self::encode),
                reset: Some(Self::reset),
                del: Some(Self::del),
            },
            encoder: Some(encoder),
        }
    }

    fn encoder(&mut self) -> &mut E {
        unsafe { self.encoder.as_mut().unwrap_unchecked() }
    }

    pub unsafe extern "C" fn encode(
        encoder: *mut rmt_encoder_t,
        tx_channel: rmt_channel_handle_t,
        primary_data: *const ::core::ffi::c_void,
        data_size: usize,
        ret_state: *mut rmt_encode_state_t,
    ) -> usize {
        let this = Self::containerof(encoder).as_mut().unwrap();

        let primary_data = core::slice::from_raw_parts(
            primary_data.cast::<E::Item>(),
            data_size / mem::size_of::<E::Item>(),
        );

        let mut channel_handle = RmtChannelHandle { handle: tx_channel };

        let (written, state) = this.encoder().encode(&mut channel_handle, primary_data);

        *ret_state = state.as_repr();

        written
    }

    pub unsafe extern "C" fn reset(encoder: *mut rmt_encoder_t) -> esp_err_t {
        let this = Self::containerof(encoder).as_mut().unwrap();

        match this.encoder().reset() {
            Ok(()) => ESP_OK,
            Err(_) => ESP_FAIL,
        }
    }

    pub unsafe extern "C" fn del(encoder: *mut rmt_encoder_t) -> esp_err_t {
        let this = Self::containerof(encoder).as_mut().unwrap();

        // drop the encoder
        this.encoder.take();

        ESP_OK
    }
}

impl<E: Encoder> RawEncoder for EncoderWrapper<E> {
    type Item = E::Item;

    fn handle(&mut self) -> &mut rmt_encoder_t {
        &mut self.0.as_mut().base
    }
}
