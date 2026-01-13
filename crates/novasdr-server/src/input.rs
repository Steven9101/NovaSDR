#[cfg(feature = "soapysdr")]
mod soapysdr;

use novasdr_core::config::{InputDriver, ReceiverConfig};
use std::io::Read;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub fn open(
    receiver: &ReceiverConfig,
    stop_requested: Arc<AtomicBool>,
) -> anyhow::Result<(Box<dyn Read + Send>, &'static str)> {
    let driver_name = receiver.input.driver.as_str();
    match &receiver.input.driver {
        InputDriver::Stdin { .. } => Ok((Box::new(std::io::stdin()), driver_name)),
        InputDriver::Fifo {
            format: _format,
            path,
        } => Ok((
            Box::new(
                std::fs::File::open(path)
                    .map_err(|e| anyhow::anyhow!("Error open file '{path}': {e}"))?,
            ),
            driver_name,
        )),
        InputDriver::SoapySdr(driver) => {
            #[cfg(feature = "soapysdr")]
            {
                Ok((
                    soapysdr::open(driver, &receiver.input, stop_requested)?,
                    driver_name,
                ))
            }

            #[cfg(not(feature = "soapysdr"))]
            {
                let _ = (driver, stop_requested);
                anyhow::bail!(
                    "SoapySDR input support is disabled (rebuild with Cargo feature \"soapysdr\")"
                )
            }
        }
    }
}
