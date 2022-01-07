use core::{
    convert::TryInto,
    fmt::{Display, Write},
};

use embedded_hal::{
    digital::v2::{InputPin, OutputPin},
    spi::FullDuplex,
};
#[cfg(feature = "genio-traits")]
use genio;
use nb;
use numtoa::NumToA;
#[cfg(feature = "genio-traits")]
use void;

use crate::{commands::*, Error, WifiNina};

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
    pub fn socket_new(&mut self) -> Result<Socket, Error<SpiError>> {
        let mut socket = InvalidSocket::new();

        self.send_and_receive(
            NinaCommand::GetSocket,
            Params::none(),
            Params::of(&mut [RecvParam::Socket(&mut socket)]),
        )?;

        Ok(socket.try_into().map_err(|_| Error::NoSocketAvailable)?)
    }

    pub fn socket_status(&mut self, socket: &Socket) -> Result<SocketStatus, Error<SpiError>> {
        let mut status: u8 = 255;

        self.send_and_receive(
            NinaCommand::GetClientStateTcp,
            Params::of(&mut [SendParam::Byte(socket.num())]),
            Params::of(&mut [RecvParam::Byte(&mut status)]),
        )?;

        Ok(status.into())
    }

    pub fn socket_open(
        &mut self,
        socket: &'_ Socket,
        protocol: Protocol,
        destination: Destination,
        port: u16,
    ) -> Result<SocketStatus, Error<SpiError>> {
        let mut result: Option<u8> = None;

        match destination {
            Destination::Ip(ip) => self.send_and_receive(
                NinaCommand::StartClientTcp,
                Params::of(&mut [
                    SendParam::Bytes(&mut ip.iter().cloned()),
                    SendParam::Word(port),
                    SendParam::Byte(socket.num()),
                    SendParam::Byte(protocol.into()),
                ]),
                Params::of(&mut [RecvParam::OptionalByte(&mut result)]),
            )?,
            Destination::Hostname(name) => self.send_and_receive(
                NinaCommand::StartClientTcp,
                Params::of(&mut [
                    SendParam::Bytes(&mut name.bytes()),
                    SendParam::Bytes(&mut [0, 0, 0, 0].iter().cloned()),
                    SendParam::Word(port),
                    SendParam::Byte(socket.num()),
                    SendParam::Byte(protocol.into()),
                ]),
                Params::of(&mut [RecvParam::OptionalByte(&mut result)]),
            )?,
        }

        if let None = result {
            return Err(Error::SocketConnectionFailed(SocketStatus::UnknownStatus));
        }

        let mut last_status = SocketStatus::UnknownStatus;

        // Wait 3 seconds for the connection.
        for _ in 0..300 {
            last_status = self.socket_status(&socket)?;

            if last_status == SocketStatus::Established {
                return Ok(SocketStatus::Established);
            }

            self.delay.delay_ms(10);
        }

        Err(Error::SocketConnectionFailed(last_status))
    }

    // Closes the socket.
    //
    // Calling "close" again on a closed socket is a no-op (as long as the chip
    // hasn’t given out the same number again, which is why we loop in here to
    // prevent code from running that might allocate a new socket).
    pub fn socket_close(&mut self, socket: &Socket) -> Result<(), Error<SpiError>> {
        self.send_and_receive(
            NinaCommand::StopClientTcp,
            Params::of(&mut [SendParam::Byte(socket.num())]),
            Params::of(&mut [RecvParam::Ack]),
        )
    }

    pub fn connect(
        &mut self,
        protocol: Protocol,
        destination: Destination,
        port: u16,
    ) -> Result<ConnectedSocket<'_, CsPin, BusyPin, Spi, SpiError, Delay>, Error<SpiError>> {
        let socket = self.socket_new()?;

        self.socket_open(&socket, protocol, destination, port)?;

        Ok(ConnectedSocket::new(self, socket))
    }

    pub fn server(&mut self, protocol: Protocol, port: u16) -> Result<Socket, Error<SpiError>> {
        let server_socket = self.socket_new()?;
        let mut result: Option<u8> = None;
        self.send_and_receive(
            NinaCommand::StartServerTcp,
            Params::of(&mut [
                SendParam::Word(port),
                SendParam::Byte(server_socket.num()),
                SendParam::Byte(protocol.into()),
            ]),
            Params::of(&mut [RecvParam::OptionalByte(&mut result)]),
        )?;

        Ok(server_socket)
    }

    pub fn select_available(
        &mut self,
        server_socket: &Socket,
    ) -> Result<ConnectedSocket<'_, CsPin, BusyPin, Spi, SpiError, Delay>, Error<SpiError>> {
        let mut client_socket: u16 = 0;

        self.send_and_receive(
            NinaCommand::AvailableDataTcp,
            Params::of(&mut [SendParam::Byte(server_socket.num())]),
            Params::of(&mut [RecvParam::LEWord(&mut client_socket)]),
        )?;

        let socket: Socket = InvalidSocket::from(client_socket as u8)
            .try_into()
            .map_err(|_| Error::NoSocketAvailable)?;

        Ok(ConnectedSocket::new(self, Socket::new(client_socket as u8)))
    }

    pub fn socket_write(
        &mut self,
        socket: &Socket,
        bytes: &mut dyn ExactSizeIterator<Item = u8>,
    ) -> Result<usize, Error<SpiError>> {
        let mut written = 0u16;

        self.send_and_receive(
            NinaCommand::SendDataTcp,
            Params::with_16_bit_length(&mut [
                SendParam::Byte(socket.num()),
                SendParam::Bytes(bytes),
            ]),
            // Yes, this comes back in little-endian rather than in network order.
            Params::of(&mut [RecvParam::LEWord(&mut written)]),
        )?;

        Ok(written as usize)
    }

    pub fn socket_read(
        &mut self,
        socket: &Socket,
        buf: &mut [u8],
    ) -> Result<usize, nb::Error<Error<SpiError>>> {
        let mut available: u16 = 0;

        self.send_and_receive(
            NinaCommand::AvailableDataTcp,
            Params::of(&mut [SendParam::Byte(socket.num())]),
            Params::of(&mut [RecvParam::LEWord(&mut available)]),
        )
        .map_err(|err| nb::Error::Other(err))?;

        if available == 0 {
            return match self.socket_status(socket)? {
                SocketStatus::Closed => Ok(0),
                _ => Err(nb::Error::WouldBlock),
            };
        }

        let req_size = core::cmp::min(available, buf.len() as u16);

        let mut read: usize = 0;

        self.send_and_receive(
            NinaCommand::GetDatabufTcp,
            Params::with_16_bit_length(&mut [
                SendParam::Byte(socket.num()),
                SendParam::LEWord(req_size),
            ]),
            Params::with_16_bit_length(&mut [RecvParam::Buffer(buf, &mut read)]),
        )
        .map_err(|err| {
            return nb::Error::Other(err);
        })?;

        Ok(read)
    }
}

// We include the Spi and the chip select in the type as a way to keep Sockets
// from being re-used across WifiNina instances.
//
// These are refs because the docs for PhantomData say to use refs when there’s
// not ownership.

pub struct InvalidSocket {
    num: u8,
}

impl InvalidSocket {
    const INVALID: u8 = 255;
    pub(crate) fn new() -> Self {
        InvalidSocket { num: Self::INVALID }
    }

    pub fn num_mut(&mut self) -> &mut u8 {
        &mut self.num
    }

    pub fn valid(num: u8) -> bool {
        num != Self::INVALID
    }
}

impl TryInto<Socket> for InvalidSocket {
    type Error = ();
    fn try_into(self) -> Result<Socket, Self::Error> {
        if Self::valid(self.num) {
            Ok(Socket::new(self.num))
        } else {
            Err(())
        }
    }
}

impl From<u8> for InvalidSocket {
    fn from(num: u8) -> Self {
        InvalidSocket { num }
    }
}

pub struct Socket {
    num: u8,
}

impl Socket {
    pub fn new(num: u8) -> Self {
        Socket { num }
    }

    pub fn num(&self) -> u8 {
        self.num
    }
}

impl core::fmt::Debug for Socket {
    fn fmt(
        &self,
        fmt: &mut core::fmt::Formatter<'_>,
    ) -> core::result::Result<(), core::fmt::Error> {
        write!(fmt, "Socket[{}]", self.num)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(u8)]
pub enum Protocol {
    TCP = 0,
    UDP = 1,
    TLS = 2,
}
impl Into<u8> for Protocol {
    fn into(self) -> u8 {
        self as u8
    }
}

pub enum Destination<'a> {
    Ip([u8; 4]),
    Hostname(&'a str),
}

impl<'a> Display for Destination<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Destination::Ip(arr) => {
                let mut buf = [0u8; 4];

                for part in arr {
                    f.write_str(part.numtoa_str(10, &mut buf))?;
                    f.write_char('.'); // yeah, I know
                }
                Ok(())
            }
            Destination::Hostname(h) => f.write_str(h),
        }
    }
}

#[repr(u8)]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SocketStatus {
    Closed = 0,
    Listen = 1,
    SynSent = 2,
    SynReceived = 3,
    Established = 4,
    FinWait1 = 5,
    FinWait2 = 6,
    CloseWait = 7,
    Closing = 8,
    LastAck = 9,
    TimeWait = 10,

    UnknownStatus = 255,
}

impl From<u8> for SocketStatus {
    fn from(s: u8) -> Self {
        match s {
            0 => SocketStatus::Closed,
            1 => SocketStatus::Listen,
            2 => SocketStatus::SynSent,
            3 => SocketStatus::SynReceived,
            4 => SocketStatus::Established,
            5 => SocketStatus::FinWait1,
            6 => SocketStatus::FinWait2,
            7 => SocketStatus::CloseWait,
            8 => SocketStatus::Closing,
            9 => SocketStatus::LastAck,
            10 => SocketStatus::TimeWait,

            _ => SocketStatus::UnknownStatus,
        }
    }
}

pub struct ConnectedSocket<'a, CS, B, S, SE, D>
where
    CS: OutputPin,
    B: InputPin,
    S: FullDuplex<u8, Error = SE> + embedded_hal::blocking::spi::Write<u8, Error = SE>,
    SE: Debug,
    D: embedded_hal::blocking::delay::DelayMs<u16>,
{
    wifi: &'a mut WifiNina<CS, B, S, D>,
    socket: Socket,
}

impl<'a, CS, B, S, SE, D> ConnectedSocket<'a, CS, B, S, SE, D>
where
    CS: OutputPin,
    B: InputPin,
    S: FullDuplex<u8, Error = SE> + embedded_hal::blocking::spi::Write<u8, Error = SE>,
    SE: Debug,
    D: embedded_hal::blocking::delay::DelayMs<u16>,
{
    pub fn new(wifi: &'a mut WifiNina<CS, B, S, D>, socket: Socket) -> Self {
        ConnectedSocket { wifi, socket }
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, nb::Error<Error<SE>>> {
        self.wifi.socket_read(&self.socket, buf)
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize, Error<SE>> {
        self.wifi
            .socket_write(&self.socket, &mut buf.iter().cloned())
    }

    pub fn socket(&self) -> &Socket {
        &self.socket
    }
}

impl<'a, CS, B, S, SE, D> Drop for ConnectedSocket<'a, CS, B, S, SE, D>
where
    CS: OutputPin,
    B: InputPin,
    S: FullDuplex<u8, Error = SE> + embedded_hal::blocking::spi::Write<u8, Error = SE>,
    SE: Debug,
    D: embedded_hal::blocking::delay::DelayMs<u16>,
{
    fn drop(&mut self) {
        self.wifi.socket_close(&self.socket).ok();
    }
}

impl<'a, CS, B, S, SE, D> core::fmt::Write for ConnectedSocket<'a, CS, B, S, SE, D>
where
    CS: OutputPin,
    B: InputPin,
    S: FullDuplex<u8, Error = SE> + embedded_hal::blocking::spi::Write<u8, Error = SE>,
    SE: Debug,
    D: embedded_hal::blocking::delay::DelayMs<u16>,
{
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        match self.write(s.as_bytes()) {
            Ok(_) => Ok(()),
            Err(_) => Err(core::fmt::Error),
        }
    }
}

#[cfg(feature = "genio-traits")]
impl<'a, CS, B, S, SE, D> genio::Read for ConnectedSocket<'a, CS, B, S, SE, D>
where
    CS: OutputPin,
    B: InputPin,
    S: FullDuplex<u8, Error = SE> + embedded_hal::blocking::spi::Write<u8, Error = SE>,
    SE: Debug,
    D: embedded_hal::blocking::delay::DelayMs<u16>,
{
    type ReadError = nb::Error<Error<SE>>;

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::ReadError> {
        self.read(buf)
    }
}

#[cfg(feature = "genio-traits")]
impl<'a, CS, B, S, SE, D> genio::Write for ConnectedSocket<'a, CS, B, S, SE, D>
where
    CS: OutputPin,
    B: InputPin,
    S: FullDuplex<u8, Error = SE> + embedded_hal::blocking::spi::Write<u8, Error = SE>,
    SE: Debug,
    D: embedded_hal::blocking::delay::DelayMs<u16>,
{
    type WriteError = Error<SE>;
    type FlushError = void::Void;

    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::WriteError> {
        self.write(buf)
    }

    fn flush(&mut self) -> Result<(), Self::FlushError> {
        Ok(())
    }

    fn size_hint(&mut self, _: usize) {}

    fn uses_size_hint(&self) -> bool {
        false
    }
}
