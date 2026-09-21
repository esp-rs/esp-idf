//! Updates keys from the NVS (default partition) using the native Rust API.
//!
//! Note that this module exposes two separate set of APIs:
//!  * the get_XXX/set_XXX API (where XXX is u8, str, etc.) - this is only for interop with C code that uses the C ESP IDF NVS API as well.
//!  * the `get_raw`/`set_raw` APIs that take a `&[u8]`. This is the "native" Rust API that implements the `RawStorage` trait from `embedded-svc`
//!    and it should be preferred actually, as you can layer on top of it any serde you want.
//!
//! More info regarding NVS:
//!   https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/storage/nvs_flash.html

use esp_idf_svc::log::EspLogger;
use esp_idf_svc::nvs::*;

use postcard::{from_bytes, to_vec};

use serde::{Deserialize, Serialize};

use log::info;

#[derive(Serialize, Deserialize, Debug)]
struct StructToBeStored<'a> {
    some_bytes: &'a [u8],
    a_str: &'a str,
    a_number: i16,
}

fn main() -> anyhow::Result<()> {
    EspLogger::initialize_default();

    let nvs_default_partition: EspNvsPartition<NvsDefault> = EspDefaultNvsPartition::take()?;

    let test_namespace = "test_ns";
    let nvs = match EspNvs::new(nvs_default_partition, test_namespace, true) {
        Ok(nvs) => {
            info!("Got namespace {test_namespace:?} from default partition");
            nvs
        }
        Err(e) => panic!("Could't get namespace {e:?}"),
    };

    let storage = EspKeyValueStorage::new(nvs);

    let key_raw_u8 = "test_raw_u8";
    {
        let key_raw_u8_data: &[u8] = &[42];

        match storage.set_raw(key_raw_u8, key_raw_u8_data) {
            Ok(_) => info!("Key updated"),
            // You can find the meaning of the error codes in the output of the error branch in:
            // https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/error-codes.html
            Err(e) => info!("Key not updated {e:?}"),
        };
    }

    {
        let key_raw_u8_data: &mut [u8] = &mut [u8::MAX];

        match storage.get_raw(key_raw_u8, key_raw_u8_data) {
            Ok(v) => match v {
                Some(vv) => info!("{key_raw_u8:?} = {vv:?}"),
                None => todo!(),
            },
            Err(e) => info!("Couldn't get key {key_raw_u8} because{e:?}"),
        };
    }

    let key_raw_str: &str = "test_raw_str";
    {
        let key_raw_str_data = "Hello from the NVS (I'm raw)!";

        match storage.set_raw(
            key_raw_str,
            &to_vec::<&str, 100>(&key_raw_str_data).unwrap(),
        ) {
            Ok(_) => info!("Key {key_raw_str} updated"),
            Err(e) => info!("Key {key_raw_str} not updated {e:?}"),
        };
    }

    {
        let key_raw_str_data: &mut [u8] = &mut [0; 100];

        match storage.get_raw(key_raw_str, key_raw_str_data) {
            Ok(v) => {
                if let Some(the_str) = v {
                    info!("{key_raw_str:?} = {:?}", from_bytes::<&str>(the_str));
                }
            }
            Err(e) => info!("Couldn't get key {key_raw_str} because {e:?}"),
        };
    }

    let key_raw_struct: &str = "test_raw_struct";
    {
        let key_raw_struct_data = StructToBeStored {
            some_bytes: &[1, 2, 3, 4],
            a_str: "I'm a str inside a struct!",
            a_number: 42,
        };

        match storage.set_raw(
            key_raw_struct,
            &to_vec::<StructToBeStored, 100>(&key_raw_struct_data).unwrap(),
        ) {
            Ok(_) => info!("Key {key_raw_struct} updated"),
            Err(e) => info!("key {key_raw_struct} not updated {e:?}"),
        };
    }

    {
        let key_raw_struct_data: &mut [u8] = &mut [0; 100];

        match storage.get_raw(key_raw_struct, key_raw_struct_data) {
            Ok(v) => {
                if let Some(the_struct) = v {
                    info!(
                        "{:?} = {:?}",
                        key_raw_struct,
                        from_bytes::<StructToBeStored>(the_struct)
                    )
                }
            }
            Err(e) => info!("Couldn't get key {key_raw_struct} because {e:?}"),
        };
    }

    Ok(())
}
