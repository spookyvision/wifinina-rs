pub mod network;
pub mod socket;
pub mod wifi;

use core::fmt::Debug;

use embedded_hal::{
    digital::v2::{InputPin, OutputPin},
    spi::FullDuplex,
};
use nb::block;

use crate::{util::spi_ext::SpiExt, Error, WifiNina};

use self::socket::InvalidSocket;

#[derive(Debug, Copy, Clone)]
#[repr(u8)]
#[allow(dead_code)]
pub enum NinaCommand {
    SetNetwork = 0x10,
    SetNetworkAndPassphrase = 0x11,
    SetKey = 0x12,
    // Test = 0x13,
    SetIpConfig = 0x14,
    SetDnsConfig = 0x15,
    SetHostname = 0x16,
    SetPowerMode = 0x17,
    SetApNetwork = 0x18,
    SetApPassphrase = 0x19,
    SetDebug = 0x1A,

    GetConnectionStatus = 0x20,
    GetIpAddress = 0x21,
    GetMacAddress = 0x22,
    GetCurrentSsid = 0x23,
    GetCurrentRssi = 0x25,
    GetCurrentEnct = 0x26,
    ScanNetworks = 0x27,
    StartServerTcp = 0x28,

    GetSocket = 0x3F,
    GetStateTcp = 0x29,
    DataSentTcp = 0x2A,
    AvailableDataTcp = 0x2B,
    GetDataTcp = 0x2C,

    StartClientTcp = 0x2D,
    StopClientTcp = 0x2E,
    GetClientStateTcp = 0x2F,

    Disconnect = 0x30,
    GetIdxRssi = 0x32,
    GetIdxEnct = 0x33,

    RequestHostByName = 0x34,
    GetHostByName = 0x35,
    StartScanNetworks = 0x36,
    GetFirmwareVersion = 0x37,
    Ping = 0x3E,

    SendDataTcp = 0x44,
    GetDatabufTcp = 0x45,

    SetEnterpriseIdent = 0x4A,
    SetEnterpriseUsername = 0x4B,
    SetEnterprisePassword = 0x4C,
    SetEnterpriseEnable = 0x4F,

    SetPinMode = 0x50,
    SetDigitalWrite = 0x51,
    SetAnalogWrite = 0x52,

    Start = 0xE0,
    End = 0xEE,
    Error = 0xEF,
}

impl Into<u8> for NinaCommand {
    fn into(self) -> u8 {
        self as u8
    }
}

#[repr(u8)]
enum NinaResponse {
    Ack = 1,

    #[allow(dead_code)]
    Error = 255,
}

impl Into<u8> for NinaResponse {
    fn into(self) -> u8 {
        self as u8
    }
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
    const REPLY_FLAG: u8 = 1 << 7;

    // Static method because it needs to be called while chip_select is mutably
    // borrowed
    fn wait_for_response_start(spi: &mut Spi, delay: &mut Delay) -> Result<(), Error<SpiError>> {
        for attempt in 0..100 {
            let byte = spi.transfer_byte().map_err(Error::spi)?;

            if byte == NinaCommand::Start.into() {
                return Ok(());
            } else if byte == NinaCommand::Error.into() {
                return Err(Error::ErrorResponse);
            }
            delay.delay_ms(1);
        }

        // for _ in timer.timeout_iter(100.ms()) {
        //     let byte = spi.transfer_byte().map_err(Error::spi)?;

        //     if byte == NinaCommand::Start.into() {
        //         return Ok(());
        //     } else if byte == NinaCommand::Error.into() {
        //         return Err(Error::ErrorResponse);
        //     }
        // }

        Err(Error::ResponseTimeout)
    }

    fn expect_byte(spi: &mut Spi, target_char: u8) -> Result<(), Error<SpiError>> {
        let v = spi.transfer_byte().map_err(Error::spi)?;

        if v == target_char {
            Ok(())
        } else {
            Err(Error::UnexpectedResponse(target_char, v))
        }
    }

    pub fn wait_for_busy(&mut self) -> Result<(), Error<SpiError>> {
        let mut spi = self.chip_select.select(&mut self.spi, &mut self.delay)?;
        Ok(())
    }

    fn send_command(
        &mut self,
        cmd: NinaCommand,
        params: Params<SendParam>,
    ) -> Result<(), Error<SpiError>> {
        let mut spi = self.chip_select.select(&mut self.spi, &mut self.delay)?;

        let cmd_byte: u8 = cmd.into();
        let mut sent_len: usize = 0;

        let use_16_bit_length = params.use_16_bit_length();

        spi.write(&[
            NinaCommand::Start.into(),
            // Pedantic to mask out the top bit, since none of the commands use it.
            cmd_byte & !Self::REPLY_FLAG,
            params.len(),
        ])
        .map_err(Error::spi)?;

        sent_len += 3;

        let mut write_len = |spi: &mut Spi, len: usize| -> Result<(), Error<SpiError>> {
            sent_len += len;

            if use_16_bit_length {
                sent_len += 2;
                spi.write(&(len as u16).to_be_bytes()).map_err(Error::spi)?;
            } else {
                sent_len += 1;
                spi.write(&[len as u8]).map_err(Error::spi)?;
            };

            Ok(())
        };

        let write_bytes: fn(&mut Spi, &mut dyn Iterator<Item = u8>) -> Result<(), Error<SpiError>> =
            |spi: &mut Spi, bytes: &mut dyn Iterator<Item = u8>| {
                for word in bytes.into_iter() {
                    block!(spi.send(word.clone())).map_err(Error::spi)?;
                    block!(spi.read()).map_err(Error::spi)?;
                }

                Ok(())
                //spi.write_iter(bytes).map_err(Error::spi)
            };

        for p in params {
            match p {
                SendParam::Byte(b) => {
                    write_len(&mut spi, 1)?;
                    write_bytes(&mut spi, &mut [*b].iter().cloned())?;
                }

                SendParam::Word(w) => {
                    write_len(&mut spi, 2)?;
                    write_bytes(&mut spi, &mut w.to_be_bytes().iter().cloned())?;
                }

                SendParam::LEWord(w) => {
                    write_len(&mut spi, 2)?;
                    write_bytes(&mut spi, &mut w.to_le_bytes().iter().cloned())?;
                }

                SendParam::Bytes(it) => {
                    write_len(&mut spi, it.len())?;
                    write_bytes(&mut spi, it)?;
                }
            };
        }

        spi.write(&[NinaCommand::End.into()]).map_err(Error::spi)?;

        sent_len += 1;

        // Pad out request to a multiple of 4 bytes.
        while sent_len % 4 != 0 {
            spi.write(&[0]).map_err(Error::spi)?;
            sent_len += 1;
        }

        Ok(())
    }

    fn receive_response(
        &mut self,
        cmd: NinaCommand,
        params: Params<RecvParam>,
    ) -> Result<(), Error<SpiError>> {
        let mut spi = self.chip_select.select(&mut self.spi, &mut self.delay)?;

        let cmd_byte: u8 = cmd.into();
        Self::wait_for_response_start(&mut spi, &mut self.delay)?;
        // We expect that the server sends back the same command, with the high bit
        // set to indicate a reply.
        Self::expect_byte(&mut spi, Self::REPLY_FLAG | cmd_byte)?;

        let use_16_bit_length = params.use_16_bit_length();

        let read_len = |spi: &mut Spi, expect: Option<usize>| -> Result<usize, Error<SpiError>> {
            let len: usize;

            if use_16_bit_length {
                let bits = [
                    spi.transfer_byte().map_err(Error::spi)?,
                    spi.transfer_byte().map_err(Error::spi)?,
                ];

                len = u16::from_be_bytes(bits) as usize;
            } else {
                len = spi.transfer_byte().map_err(Error::spi)? as usize;
            };

            if let Some(expect) = expect {
                if len != expect {
                    return Err(Error::MismatchedParamSize(expect, len));
                }
            }

            return Ok(len);
        };

        let param_count: u8 = spi.transfer_byte().map_err(Error::spi)?;
        let mut param_idx: u8 = 0;

        for param_handler in params {
            if param_idx == param_count {
                match param_handler {
                    RecvParam::OptionalByte(_) => continue,
                    _ => return Err(Error::MissingParam(param_idx)),
                }
            };

            match param_handler {
                RecvParam::Ack => {
                    read_len(&mut spi, Some(1))?;
                    Self::expect_byte(&mut spi, NinaResponse::Ack.into())?;
                }

                RecvParam::ExpectByte(b) => {
                    read_len(&mut spi, Some(1))?;
                    Self::expect_byte(&mut spi, *b)?;
                }

                RecvParam::Byte(ref mut b) => {
                    read_len(&mut spi, Some(1))?;
                    **b = spi.transfer_byte().map_err(Error::spi)?;
                }

                RecvParam::OptionalByte(ref mut op) => {
                    read_len(&mut spi, Some(1))?;
                    op.replace(spi.transfer_byte().map_err(Error::spi)?);
                }

                RecvParam::Word(ref mut w) => {
                    read_len(&mut spi, Some(2))?;

                    let bits = [
                        spi.transfer_byte().map_err(Error::spi)?,
                        spi.transfer_byte().map_err(Error::spi)?,
                    ];

                    **w = u16::from_be_bytes(bits);
                }

                RecvParam::LEWord(ref mut w) => {
                    read_len(&mut spi, Some(2))?;

                    let bits = [
                        spi.transfer_byte().map_err(Error::spi)?,
                        spi.transfer_byte().map_err(Error::spi)?,
                    ];

                    **w = u16::from_le_bytes(bits);
                }

                RecvParam::ByteArray(arr) => {
                    read_len(&mut spi, Some(arr.len()))?;

                    for i in 0..arr.len() {
                        arr[i] = spi.transfer_byte().map_err(Error::spi)?;
                    }
                }

                RecvParam::Buffer(arr, ref mut len) => {
                    **len = read_len(&mut spi, None)?;

                    for i in 0..**len {
                        arr[i] = spi.transfer_byte().map_err(Error::spi)?;
                    }
                }

                RecvParam::Socket(ref mut socket) => {
                    read_len(&mut spi, Some(1))?;
                    *socket.num_mut() = spi.transfer_byte().map_err(Error::spi)?;
                }
            };

            param_idx += 1;
        }

        if param_count > param_idx {
            return Err(Error::UnexpectedParam(param_idx));
        }

        Ok(())
    }

    fn send_and_receive(
        &mut self,
        command: NinaCommand,
        send_params: Params<SendParam>,
        recv_params: Params<RecvParam>,
    ) -> Result<(), Error<SpiError>> {
        self.send_command(command, send_params)?;
        self.receive_response(command, recv_params)
    }

    pub fn set_debug(&mut self, enabled: bool) -> Result<(), Error<SpiError>> {
        self.send_and_receive(
            NinaCommand::SetDebug,
            Params::of(&mut [SendParam::Byte(enabled as u8)]),
            Params::of(&mut [RecvParam::Ack]),
        )
    }
}

pub enum SendParam<'a> {
    Byte(u8),
    Word(u16),
    LEWord(u16),
    Bytes(&'a mut dyn ExactSizeIterator<Item = u8>),
}

#[allow(dead_code)]
pub enum RecvParam<'a> {
    Ack,
    Byte(&'a mut u8),
    Socket(&'a mut InvalidSocket),
    OptionalByte(&'a mut Option<u8>),
    ExpectByte(u8),
    Word(&'a mut u16),
    LEWord(&'a mut u16),
    ByteArray(&'a mut [u8]),
    Buffer(&'a mut [u8], &'a mut usize),
}

pub struct Params<'a, P> {
    params: &'a mut [P],
    use_16_bit_length: bool,
}

impl<'a, P> Params<'a, P> {
    pub fn none() -> Self {
        Params {
            params: &mut [],
            use_16_bit_length: false,
        }
    }

    pub fn of(params: &'a mut [P]) -> Self {
        Params {
            params,
            use_16_bit_length: false,
        }
    }

    pub fn with_16_bit_length(params: &'a mut [P]) -> Self {
        Params {
            params,
            use_16_bit_length: true,
        }
    }

    pub fn len(&self) -> u8 {
        self.params.len() as u8
    }

    pub fn use_16_bit_length(&self) -> bool {
        self.use_16_bit_length
    }
}

impl<'a, P> core::iter::IntoIterator for Params<'a, P> {
    type Item = &'a mut P;
    type IntoIter = core::slice::IterMut<'a, P>;

    fn into_iter(self) -> core::slice::IterMut<'a, P> {
        self.params.into_iter()
    }
}
