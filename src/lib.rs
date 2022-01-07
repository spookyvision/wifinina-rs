#![no_std]

mod chip_select;
pub mod commands;
pub mod util;

use core::fmt::Debug;

use chip_select::*;
use commands::{socket::SocketStatus, wifi::WifiStatus};
use embedded_hal::{
    digital::v2::{InputPin, OutputPin},
    spi::FullDuplex,
};

pub struct WifiNina<CsPin, BusyPin, Spi, Delay>
where
    CsPin: OutputPin,
    BusyPin: InputPin,
    Delay: embedded_hal::blocking::delay::DelayMs<u16>,
{
    spi: Spi,
    chip_select: WifiNinaChipSelect<Spi, CsPin, BusyPin>,
    delay: Delay,
}

impl<CsPin, BusyPin, Spi, SpiError, Delay> WifiNina<CsPin, BusyPin, Spi, Delay>
where
    BusyPin: InputPin,
    CsPin: OutputPin,
    Spi:
        FullDuplex<u8, Error = SpiError> + embedded_hal::blocking::spi::Write<u8, Error = SpiError>,
    SpiError: Debug,
    Delay: embedded_hal::blocking::delay::DelayMs<u16>, //+ embedded_hal::blocking::spi::WriteIter<u8, Error = SpiError>,
{
    // const ConnectionDelayMs: u16 = 100;

    // We take the spi here just to allow the type to be implied.
    //
    // Also resets the WifiNINA chip.
    pub fn new<ResetPin>(
        spi: Spi,
        cs: CsPin,
        busy: BusyPin,
        reset: &mut ResetPin,
        delay: Delay,
    ) -> Result<Self, Error<SpiError>>
    where
        ResetPin: OutputPin,
    {
        let mut wifi = WifiNina {
            spi,
            chip_select: WifiNinaChipSelect::new(cs, busy)
                .map_err(|_| Error::ChipSelectPinError)?,
            delay,
        };

        wifi.reset(reset)?;

        Ok(wifi)
    }

    pub fn reset<ResetPin>(&mut self, reset: &mut ResetPin) -> Result<(), Error<SpiError>>
    where
        ResetPin: OutputPin,
    {
        reset.set_low().map_err(|_| Error::ResetPinError)?;

        self.delay.delay_ms(250);

        reset.set_high().map_err(|_| Error::ResetPinError)?;

        self.delay.delay_ms(750);

        Ok(())
    }

    // Static method because it needs to be called while device_selector is borrowed
}

#[derive(Debug)]
pub enum Error<SpiError: Debug> {
    ChipSelectPinError,
    ChipSelectTimeout,

    ResponseTimeout,
    MissingParam(u8),
    UnexpectedParam(u8),
    MismatchedParamSize(usize, usize),
    ErrorResponse,
    UnexpectedResponse(u8, u8),

    ConnectionFailed(WifiStatus),
    ConnectionTimeout,

    SocketConnectionFailed(SocketStatus),
    SocketClosed,
    SocketTimeout,
    NoSocketAvailable,

    SpiError(SpiError),
    ResetPinError,
}

impl<SpiError> Error<SpiError>
where
    SpiError: Debug,
{
    // Convenience function for passing to map_err, because we can’t use
    // the From trait because SpiError is fully parameterized.
    fn spi(err: SpiError) -> Error<SpiError> {
        Error::SpiError(err)
    }
}

impl<BE, CE, SE> From<WifiNinaChipSelectError<BE, CE>> for Error<SE>
where
    SE: Debug,
{
    fn from(err: WifiNinaChipSelectError<BE, CE>) -> Self {
        match err {
            WifiNinaChipSelectError::BusyPinError(_) => Error::ChipSelectPinError,
            WifiNinaChipSelectError::CsPinError(_) => Error::ChipSelectPinError,
            WifiNinaChipSelectError::DeviceReadyTimeout => Error::ChipSelectTimeout,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
