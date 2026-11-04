use std::fs::File;
use std::io::{self, Read as _};

use nix::sys::termios::{self, SetArg, Termios};

use super::interactive::ByteSource;

#[derive(Debug, thiserror::Error)]
pub enum RawModeError {
    #[error("failed to open /dev/tty: {0}")]
    Open(#[source] io::Error),
    #[error("failed to read terminal attributes: {0}")]
    GetAttr(#[source] nix::Error),
    #[error("failed to set terminal attributes: {0}")]
    SetAttr(#[source] nix::Error),
}

pub trait TermiosBackend {
    type Attrs: Clone;

    fn get_attrs(&mut self) -> Result<Self::Attrs, RawModeError>;
    fn set_attrs(&mut self, attrs: &Self::Attrs) -> Result<(), RawModeError>;
    fn make_raw(&self, attrs: &mut Self::Attrs);
    fn read_byte(&mut self) -> io::Result<u8>;
}

pub struct TtyTermiosBackend {
    tty: File,
}

impl TermiosBackend for TtyTermiosBackend {
    type Attrs = Termios;

    fn get_attrs(&mut self) -> Result<Termios, RawModeError> {
        termios::tcgetattr(&self.tty).map_err(RawModeError::GetAttr)
    }

    fn set_attrs(&mut self, attrs: &Termios) -> Result<(), RawModeError> {
        termios::tcsetattr(&self.tty, SetArg::TCSADRAIN, attrs).map_err(RawModeError::SetAttr)
    }

    fn make_raw(&self, attrs: &mut Termios) {
        termios::cfmakeraw(attrs);
    }

    fn read_byte(&mut self) -> io::Result<u8> {
        let mut buf = [0_u8; 1];
        self.tty.read_exact(&mut buf)?;
        Ok(buf[0])
    }
}

pub struct RawModeGuard<B: TermiosBackend> {
    backend: B,
    original: B::Attrs,
}

impl<B: TermiosBackend> RawModeGuard<B> {
    pub fn enable(mut backend: B) -> Result<Self, RawModeError> {
        let original = backend.get_attrs()?;
        let mut raw = original.clone();
        backend.make_raw(&mut raw);
        backend.set_attrs(&raw)?;
        Ok(Self { backend, original })
    }
}

impl<B: TermiosBackend> ByteSource for RawModeGuard<B> {
    fn next_byte(&mut self) -> io::Result<u8> {
        self.backend.read_byte()
    }
}

impl<B: TermiosBackend> Drop for RawModeGuard<B> {
    fn drop(&mut self) {
        drop(self.backend.set_attrs(&self.original));
    }
}

pub type TtyRawMode = RawModeGuard<TtyTermiosBackend>;

impl TtyRawMode {
    pub fn open() -> Result<Self, RawModeError> {
        Self::enable(TtyTermiosBackend {
            tty: File::open("/dev/tty").map_err(RawModeError::Open)?,
        })
    }
}

mod tests;
