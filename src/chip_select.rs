use embedded_hal::digital::v2::{InputPin, OutputPin};

use crate::util::safe_spi::{ChipSelect, SafeSpi};

#[derive(Debug)]
pub enum WifiNinaChipSelectError<CsPinError, BusyPinError> {
    CsPinError(CsPinError),
    BusyPinError(BusyPinError),
    DeviceReadyTimeout,
}

// A ChipSelect implementation that listens to the ESP32’s "busy" output
// and only returns selected when it’s indictating that the device is
// ready to listen.
//
// Its select method needs a timer in order to fail if the device isn’t
// ready by a deadline.
pub struct WifiNinaChipSelect<S, CsPin: OutputPin, BusyPin: InputPin> {
    spi: core::marker::PhantomData<S>,

    cs: CsPin,
    busy: BusyPin,

    last_deselect_err: Option<WifiNinaChipSelectError<CsPin::Error, BusyPin::Error>>,
}

impl<S, CsPin, BusyPin> WifiNinaChipSelect<S, CsPin, BusyPin>
where
    CsPin: OutputPin,
    BusyPin: InputPin,
{
    // Drives the CS pin high on init
    pub fn new(
        mut cs: CsPin,
        busy: BusyPin,
    ) -> Result<Self, WifiNinaChipSelectError<CsPin::Error, BusyPin::Error>> {
        cs.set_high()
            .map_err(|err| WifiNinaChipSelectError::CsPinError(err))?;

        Ok(WifiNinaChipSelect {
            spi: core::marker::PhantomData,
            cs,
            busy,
            last_deselect_err: None,
        })
    }

    pub fn select<'a>(
        &'a mut self,
        spi: &'a mut S,
        delay: &mut impl embedded_hal::blocking::delay::DelayMs<u16>,
    ) -> Result<SafeSpi<'a, S, Self>, WifiNinaChipSelectError<CsPin::Error, BusyPin::Error>> {
        self.wait_for_busy(delay, 10_000, false)?;

        self.cs
            .set_low()
            .map_err(|err| WifiNinaChipSelectError::CsPinError(err))?;

        self.wait_for_busy(delay, 1_000, true)?;

        Ok(SafeSpi::new(spi, self))
    }

    fn wait_for_busy(
        &mut self,
        delay: &mut impl embedded_hal::blocking::delay::DelayMs<u16>,
        timeout: u16,
        val: bool,
    ) -> Result<(), WifiNinaChipSelectError<CsPin::Error, BusyPin::Error>> {
        for attempt in 0..timeout {
            match self.busy.is_high() {
                Ok(b) => {
                    if b == val {
                        return Ok(());
                    }
                }
                Err(err) => return Err(WifiNinaChipSelectError::BusyPinError(err)),
            }
            delay.delay_ms(1);
        }
        // for _ in timer.timeout_iter(timeout) {
        //     match self.busy.is_high() {
        //         Ok(b) => {
        //             if b == val {
        //                 return Ok(());
        //             }
        //         }
        //         Err(err) => return Err(WifiNinaChipSelectError::BusyPinError(err)),
        //     }
        // }

        Err(WifiNinaChipSelectError::DeviceReadyTimeout)
    }
}

impl<S, CsPin, BusyPin> ChipSelect for WifiNinaChipSelect<S, CsPin, BusyPin>
where
    CsPin: OutputPin,
    BusyPin: InputPin,
{
    type Spi = S;

    fn deselect(&mut self) {
        self.last_deselect_err = self
            .cs
            .set_high()
            .map_err(|err| WifiNinaChipSelectError::CsPinError(err))
            .err();
    }
}
