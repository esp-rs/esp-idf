mod copy_encoder;
pub use copy_encoder::*;
mod bytes_encoder;
pub use bytes_encoder::*;

#[cfg(esp_idf_version_at_least_5_3_0)]
#[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_3_0)))]
pub mod simple_encoder;

use alloc::boxed::Box;
use core::mem;

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

/// RMT encoding state.
///
/// `rmt_encode_state_t` (see `rmt_encoder.h`) is a genuine C bitmask, not a plain enum:
/// `RMT_ENCODING_RESET = 0`, `RMT_ENCODING_COMPLETE = (1 << 0)`,
/// `RMT_ENCODING_MEM_FULL = (1 << 1)`, `RMT_ENCODING_WITH_EOF = (1 << 2)`, and the real driver is
/// free to OR any of these together in one return value. This used to be modeled as a set of
/// mutually-exclusive enum variants, which is fundamentally the wrong shape for a bitmask type:
/// converting a real, valid combined value back from C hit an unreachable-in-theory wildcard arm
/// and panicked - and because this conversion runs inside the RMT ISR (see [`Encoder::encode`]'s
/// own "ISR Safety" note), a Rust panic can't safely lock/print there, so ESP-IDF's own newlib
/// lock code detects the invalid ISR-context lock attempt and calls `abort()` instead of printing
/// a readable panic message - a real, live-hardware-reproduced crash, not a theoretical concern.
/// Confirmed (via a temporary ISR-safe `esp_rom_printf` probe, since normal panic output is what
/// breaks in this exact ISR-context scenario) that combined values are not an edge case: a small
/// WS2812-strip transmission that finishes encoding at the same time it exactly fills the
/// remaining RMT memory block reports `RMT_ENCODING_COMPLETE | RMT_ENCODING_MEM_FULL` (value `3`)
/// on real hardware - a different combination than the also-valid `COMPLETE | WITH_EOF`, and not
/// necessarily the only one either. A plain enum cannot represent an unbounded set of flag
/// combinations without either growing a variant per combination forever or panicking on whichever
/// one it hasn't special-cased yet, which is exactly the bug this type had. Modeled as a thin
/// bitmask wrapper instead, matching the C type it mirrors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderState(rmt_encode_state_t);

impl EncoderState {
    /// The encoding session is in reset state.
    pub const RESET: Self = Self(rmt_encode_state_t_RMT_ENCODING_RESET);
    /// The encoding session is finished, the caller can continue with subsequent encoding.
    pub const COMPLETE: Self = Self(rmt_encode_state_t_RMT_ENCODING_COMPLETE);
    /// The encoding artifact memory is full, the caller should return from current encoding session.
    pub const MEM_FULL: Self = Self(rmt_encode_state_t_RMT_ENCODING_MEM_FULL);
    /// The encoding session has inserted the EOF marker to the symbol stream.
    #[cfg(esp_idf_version_at_least_5_5_0)]
    #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_5_0)))]
    pub const WITH_EOF: Self = Self(rmt_encode_state_t_RMT_ENCODING_WITH_EOF);

    /// Whether this is the reset state (no other flag can be combined with it - `RESET` is `0`).
    #[must_use]
    pub const fn is_reset(&self) -> bool {
        self.0 == rmt_encode_state_t_RMT_ENCODING_RESET
    }

    /// Whether the [`COMPLETE`](Self::COMPLETE) flag is set - may also have
    /// [`MEM_FULL`](Self::MEM_FULL) and/or [`WITH_EOF`](Self::WITH_EOF) set at the same time.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.0 & rmt_encode_state_t_RMT_ENCODING_COMPLETE != 0
    }

    /// Whether the [`MEM_FULL`](Self::MEM_FULL) flag is set - may also have
    /// [`COMPLETE`](Self::COMPLETE) set at the same time (the last chunk of a session can both
    /// finish encoding and exactly fill the remaining memory block in the same call).
    #[must_use]
    pub const fn is_mem_full(&self) -> bool {
        self.0 & rmt_encode_state_t_RMT_ENCODING_MEM_FULL != 0
    }

    /// Whether the [`WITH_EOF`](Self::WITH_EOF) flag is set.
    #[cfg(esp_idf_version_at_least_5_5_0)]
    #[cfg_attr(feature = "nightly", doc(cfg(esp_idf_version_at_least_5_5_0)))]
    #[must_use]
    pub const fn has_eof(&self) -> bool {
        self.0 & rmt_encode_state_t_RMT_ENCODING_WITH_EOF != 0
    }
}

impl core::ops::BitOr for EncoderState {
    type Output = Self;

    /// Combines two encoding-state flags, e.g. `EncoderState::COMPLETE | EncoderState::WITH_EOF`.
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl From<EncoderState> for rmt_encode_state_t {
    fn from(value: EncoderState) -> Self {
        value.0
    }
}

impl From<rmt_encode_state_t> for EncoderState {
    fn from(value: rmt_encode_state_t) -> Self {
        Self(value)
    }
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
    /// The supported states are listed in [`EncoderState`]. If the result has
    /// [`EncoderState::is_complete`] true, it means the current encoder has finished work (this
    /// may be combined with either of the flags below in the same result - see [`EncoderState`]'s
    /// own doc comment).
    ///
    /// If the result has [`EncoderState::is_mem_full`] true, the program needs to yield from the
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
    ) -> (usize, EncoderState);

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
    ) -> (usize, EncoderState) {
        let encoder_handle = self.handle();
        let Some(encode) = encoder_handle.encode else {
            return (0, EncoderState::RESET);
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

        (written, ret_state.into())
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

        *ret_state = state.into();

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
