use core::convert::Infallible;

use embassy_lora::iv::{InterruptHandler, Stm32wlInterfaceVariant};
use embassy_stm32::flash::{Blocking, Flash};
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::peripherals::{PC4, RNG, SUBGHZSPI};
use embassy_stm32::rng::Rng;
use embassy_stm32::spi::Spi;
use embassy_stm32::{bind_interrupts, pac, Peripherals};
use embassy_time::Delay;
use lora_phy::mod_params::BoardType;
use lora_phy::sx1261_2::SX1261_2;
use lora_phy::LoRa;
use lorawan::device::non_volatile_store::NonVolatileStore;
use lorawan::device::Device;
use lorawan::mac::region::Region;
use lorawan::mac::MacDevice;

use crate::lora_radio::LoRaRadio;
use crate::timer::LoraTimer;
use rand_core::RngCore;

bind_interrupts!(struct Irqs{
    SUBGHZ_RADIO => InterruptHandler;
});

extern "C" {
    static __storage: u8;
}
pub struct LoraDevice<'d> {
    rng: DeviceRng<'d>,
    radio: LoRaRadio<'d>,
    timer: LoraTimer,
    non_volatile_store: DeviceNonVolatileStore<'d>,
}
impl<'a> LoraDevice<'a> {
    pub async fn new(peripherals: Peripherals) -> LoraDevice<'a> {
        let lora: LoRa<
            SX1261_2<Spi<'a, SUBGHZSPI, _, _>, Stm32wlInterfaceVariant<Output<'a, PC4>>>,
            Delay,
        > = {
            let spi = Spi::new_subghz(
                peripherals.SUBGHZSPI,
                peripherals.DMA1_CH2,
                peripherals.DMA1_CH3,
            );
            let iv = Stm32wlInterfaceVariant::new(
                Irqs,
                None,
                Some(Output::new(peripherals.PC4, Level::Low, Speed::High)),
            )
            .unwrap();

            LoRa::new(
                SX1261_2::new(BoardType::Stm32wlSx1262, spi, iv),
                true,
                Delay,
            )
            .await
            .unwrap()
        };
        let radio = LoRaRadio::new(lora);
        let non_volatile_store =
            DeviceNonVolatileStore::new(Flash::new_blocking(peripherals.FLASH));
        let ret = Self {
            rng: DeviceRng(Rng::new(peripherals.RNG)),
            radio,
            timer: LoraTimer::new(),
            non_volatile_store,
        };
        ret
    }
}
impl<'d> defmt::Format for LoraDevice<'d> {
    fn format(&self, fmt: defmt::Formatter<'_>) {
        defmt::write!(fmt, "LoraDevice")
    }
}
pub struct DeviceRng<'a>(Rng<'a, RNG>);

pub struct DeviceNonVolatileStore<'a> {
    flash: Flash<'a, Blocking>,
    buf: [u8; 256],
}
impl<'a> DeviceNonVolatileStore<'a> {
    pub fn new(flash: Flash<'a, Blocking>) -> Self {
        Self {
            flash,
            buf: [0xFF; 256],
        }
    }
    pub fn offset() -> u32 {
        (unsafe { &__storage as *const u8 as u32 }) - pac::FLASH_BASE as u32
    }
}
#[derive(Debug, PartialEq, defmt::Format)]
pub enum NonVolatileStoreError {
    Flash(embassy_stm32::flash::Error),
    Encoding,
}
impl<'m> NonVolatileStore for DeviceNonVolatileStore<'m> {
    type Error = NonVolatileStoreError;

    fn save<'a, T>(&mut self, item: T) -> Result<(), Self::Error>
    where
        T: Sized + Into<&'a [u8]>,
    {
        self.buf = [0xFFu8; 256];
        let offset = Self::offset();
        self.flash
            .blocking_erase(offset, offset + 2048)
            .map_err(NonVolatileStoreError::Flash)?;
        self.buf[..core::mem::size_of::<T>()].copy_from_slice(item.into());
        self.flash
            .blocking_write(offset, &self.buf)
            .map_err(NonVolatileStoreError::Flash)
    }

    fn load<'a, T>(&'a mut self) -> Result<T, Self::Error>
    where
        T: Sized + TryFrom<&'a [u8]>,
    {
        let offset = Self::offset();
        self.flash
            .read(offset, &mut self.buf)
            .map_err(NonVolatileStoreError::Flash)?;
        self.buf[..core::mem::size_of::<T>()]
            .try_into()
            .map_err(|_| NonVolatileStoreError::Encoding)
    }
}

impl<'a> lorawan::device::rng::Rng for DeviceRng<'a> {
    type Error = Infallible;

    fn next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(self.0.next_u32())
    }
}
impl<'a> Device for LoraDevice<'a> {
    type Timer = LoraTimer;

    type Radio = LoRaRadio<'a>;

    type Rng = DeviceRng<'a>;

    type NonVolatileStore = DeviceNonVolatileStore<'a>;

    fn timer(&mut self) -> &mut Self::Timer {
        &mut self.timer
    }

    fn radio(&mut self) -> &mut Self::Radio {
        &mut self.radio
    }

    fn rng(&mut self) -> &mut Self::Rng {
        &mut self.rng
    }

    fn non_volatile_store(&mut self) -> &mut Self::NonVolatileStore {
        &mut self.non_volatile_store
    }

    fn max_eirp() -> u8 {
        22
    }

    fn adaptive_data_rate_enabled(&self) -> bool {
        true
    }

    fn handle_device_time(&mut self, _seconds: u32, _nano_seconds: u32) {
        // default do nothing
    }

    fn handle_link_check(&mut self, _gateway_count: u8, _margin: u8) {
        // default do nothing
    }

    fn battery_level(&self) -> Option<f32> {
        None
    }
}
impl<'a, R> MacDevice<R, DeviceSpecs> for LoraDevice<'a> where R: Region {}

pub struct DeviceSpecs;
impl lorawan::device::DeviceSpecs for DeviceSpecs {}
