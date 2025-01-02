use embassy_stm32::{gpio::Output, mode::Async, spi::Spi};
use embassy_time::Delay;
use lora_phy::sx126x::{Stm32wl, Sx126x};
use lora_phy::LoRa;

use crate::iv::{Stm32wlInterfaceVariant, SubghzSpiDevice};

pub type LoraRadioKind<'a> =
    Sx126x<SubghzSpiDevice<Spi<'a, Async>>, Stm32wlInterfaceVariant<Output<'a>>, Stm32wl>;
pub type LoraType<'d> = LoRa<LoraRadioKind<'d>, Delay>;
