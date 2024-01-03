// use super::radio::{
//     Bandwidth, CodingRate, PhyRxTx, RfConfig, RxQuality, SpreadingFactor, TxConfig,
// };
// use super::region::constants::DEFAULT_DBM;
// use super::Timings;

use embassy_stm32::{
    gpio::{AnyPin, Output},
    peripherals::{DMA1_CH2, DMA1_CH3},
};
use embassy_time::Delay;
use lora_phy::mod_params::RadioError;
use lora_phy::sx1261_2::SX1261_2;
use lora_phy::LoRa;
use lorawan::device::radio::types::RxQuality;
use lorawan::device::radio::Radio;

use crate::iv::{Stm32wlInterfaceVariant, SubGhzSpiDevice};

pub type LoraType<'d> = LoRa<
    SX1261_2<SubGhzSpiDevice<'d, DMA1_CH2, DMA1_CH3>, Stm32wlInterfaceVariant<Output<'d, AnyPin>>>,
    Delay,
>;

/// LoRa radio using the physical layer API in the external lora-phy crate
pub struct LoRaRadio<'d> {
    pub(crate) lora: LoraType<'d>,
}

impl<'d> LoRaRadio<'d> {
    pub fn new(lora: LoraType<'d>) -> Self {
        Self { lora }
    }
}

/// Provide the LoRa physical layer rx/tx interface for boards supported by the external lora-phy crate
impl<'d> Radio for LoRaRadio<'d> {
    type Error = RadioError;

    async fn tx(
        &mut self,
        config: lorawan::device::radio::types::TxConfig,
        buf: &[u8],
    ) -> Result<usize, <LoRaRadio<'d> as lorawan::device::radio::Radio>::Error> {
        let mdltn_params = self.lora.create_modulation_params(
            config.rf.data_rate.spreading_factor,
            config.rf.data_rate.bandwidth,
            config.rf.coding_rate,
            config.rf.frequency,
        )?;
        let mut tx_pkt_params =
            self.lora.create_tx_packet_params(8, false, true, false, &mdltn_params)?;

        self.lora.prepare_for_tx(&mdltn_params, config.pw.into(), false).await?;
        self.lora.tx(&mdltn_params, &mut tx_pkt_params, buf, 0xffffff).await?;
        Ok(0)
    }

    async fn rx(
        &mut self,
        config: lorawan::device::radio::types::RfConfig,
        window_in_secs: u8,
        rx_buf: &mut [u8],
    ) -> Result<
        (usize, lorawan::device::radio::types::RxQuality),
        <LoRaRadio<'d> as lorawan::device::radio::Radio>::Error,
    > {
        let mdltn_params = self.lora.create_modulation_params(
            config.data_rate.spreading_factor,
            config.data_rate.bandwidth,
            config.coding_rate,
            config.frequency,
        )?;
        let rx_pkt_params = self.lora.create_rx_packet_params(
            8,
            false,
            rx_buf.len() as u8,
            true,
            true,
            &mdltn_params,
        )?;
        self.lora
            .prepare_for_rx(
                &mdltn_params,
                &rx_pkt_params,
                Some(window_in_secs),
                None,
                true,
                //4,
                //0x00ffffffu32,
            )
            .await?;
        match self.lora.rx(&rx_pkt_params, rx_buf).await {
            Ok((received_len, rx_pkt_status)) => {
                Ok((
                    received_len as usize,
                    RxQuality::new(rx_pkt_status.rssi, rx_pkt_status.snr as i8), // downcast snr
                ))
            }
            Err(err) => Err(err),
        }
    }

    async fn sleep(
        &mut self,
        _warm_start: bool,
    ) -> Result<(), <LoRaRadio<'d> as lorawan::device::radio::Radio>::Error> {
        Ok(())
    }
}
