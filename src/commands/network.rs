use embedded_hal::{
    digital::v2::{InputPin, OutputPin},
    spi::FullDuplex,
};

use crate::{commands::*, Error, WifiNina};

#[derive(Debug, Default)]
pub struct NetworkInfo {
    pub ip: [u8; 4],
    pub netmask: [u8; 4],
    pub gateway_ip: [u8; 4],
}

impl<CsPin, BusyPin, Spi, SpiError, Delay> WifiNina<CsPin, BusyPin, Spi, Delay>
where
    BusyPin: InputPin,
    CsPin: OutputPin,
    Spi:
        FullDuplex<u8, Error = SpiError> + embedded_hal::blocking::spi::Write<u8, Error = SpiError>,
    SpiError: Debug,
    //+ embedded_hal::blocking::spi::WriteIter<u8, Error = SpiError>,
    Delay: embedded_hal::blocking::delay::DelayMs<u16>,
{
    pub fn network_info(&mut self) -> Result<NetworkInfo, Error<SpiError>> {
        let mut network_info: NetworkInfo = Default::default();

        self.send_and_receive(
            NinaCommand::GetIpAddress,
            Params::none(),
            Params::of(&mut [
                RecvParam::ByteArray(&mut network_info.ip),
                RecvParam::ByteArray(&mut network_info.netmask),
                RecvParam::ByteArray(&mut network_info.gateway_ip),
            ]),
        )?;

        Ok(network_info)
    }

    pub fn resolve_host_name(&mut self, name: &str) -> Result<[u8; 4], Error<SpiError>> {
        let mut ip = [0u8; 4];

        self.send_and_receive(
            NinaCommand::RequestHostByName,
            Params::of(&mut [SendParam::Bytes(&mut name.bytes())]),
            Params::of(&mut [RecvParam::Ack]),
        )?;

        self.send_and_receive(
            NinaCommand::GetHostByName,
            Params::none(),
            Params::of(&mut [RecvParam::ByteArray(&mut ip)]),
        )?;

        Ok(ip)
    }
}
