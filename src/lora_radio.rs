// use super::radio::{
//     Bandwidth, CodingRate, PhyRxTx, RfConfig, RxQuality, SpreadingFactor, TxConfig,
// };
// use super::region::constants::DEFAULT_DBM;
// use super::Timings;

use embedded_hal_async::delay::DelayUs;
use lora_phy::LoRa;
use lora_phy::{mod_params::RadioError, mod_traits::RadioKind};
use lorawan::device::radio::types::RxQuality;
use lorawan::device::radio::Radio;

/// LoRa radio using the physical layer API in the external lora-phy crate
pub struct LoRaRadio<RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    pub(crate) lora: LoRa<RK, DLY>,
}

impl<RK, DLY> LoRaRadio<RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    pub fn new(lora: LoRa<RK, DLY>) -> Self {
        Self { lora }
    }
}

/// Provide the LoRa physical layer rx/tx interface for boards supported by the external lora-phy crate
impl<RK, DLY> Radio for LoRaRadio<RK, DLY>
where
    RK: RadioKind,
    DLY: DelayUs,
{
    type Error = RadioError;

    async fn tx(
        &mut self,
        config: lorawan::device::radio::types::TxConfig,
        buf: &[u8],
    ) -> Result<usize, <LoRaRadio<RK, DLY> as lorawan::device::radio::Radio>::Error> {
        let sf = config.rf.data_rate.spreading_factor.into();
        let bw = config.rf.data_rate.bandwidth.into();
        let cr = config.rf.coding_rate.into();
        let mdltn_params = self
            .lora
            .create_modulation_params(sf, bw, cr, config.rf.frequency)?;
        let mut tx_pkt_params =
            self.lora
                .create_tx_packet_params(8, false, true, false, &mdltn_params)?;

        self.lora
            .prepare_for_tx(&mdltn_params, config.pw.into(), false)
            .await?;
        self.lora
            .tx(&mdltn_params, &mut tx_pkt_params, buf, 0xffffff)
            .await?;
        Ok(0)
    }

    async fn rx(
        &mut self,
        config: lorawan::device::radio::types::RfConfig,
        rx_buf: &mut [u8],
    ) -> Result<
        (usize, lorawan::device::radio::types::RxQuality),
        <LoRaRadio<RK, DLY> as lorawan::device::radio::Radio>::Error,
    > {
        let sf = config.data_rate.spreading_factor.into();
        let bw = config.data_rate.bandwidth.into();
        let cr = config.coding_rate.into();
        let mdltn_params = self
            .lora
            .create_modulation_params(sf, bw, cr, config.frequency)?;
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
                None,
                true, // RX continuous
                false,
                4,
                0x00ffffffu32,
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
}
