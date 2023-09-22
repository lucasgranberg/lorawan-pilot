# LoRaWAN Demo Application for EU868 using an STM32WL board

## Work In Progress - transition to task-based usage of the LoRaWAN implementation.

### Description

Demonstrates a basic LoRaWAN application for:

- <a href="https://github.com/lucasgranberg/lorawan">LoRaWAN Class A</a>
- <a href="https://github.com/embassy-rs/lora-phy">LoRa physical layer</a>
- <a href="https://github.com/embassy-rs/embassy">Embassy embedded framework</a>

with the following test setup:

- <a href="https://resources.lora-alliance.org/technical-specifications/rp002-1-0-4-regional-parameters">EU868 regional parameters</a>
- <a href="https://www.st.com/en/microcontrollers-microprocessors/stm32wl-series.html">STM32WL with a Semtech SX1262 LoRa chip</a>

The Cargo.toml patch.crates-io specification includes lorawan and embassy repositories and versions that are known to work for this example.

### Code modifications needed

- in your local lorawan-pilot clone, git checkout the eu868-stm32wl-task branch.
- in main.rs, setup the LoRaRadio for your specific LoRa chip or board.
- in main.rs, set the dev_eui, app_eui, and app_key configured for your device and gateway.

### Suggested setup

The following can all be run from your local lorawan-pilot folder, with the eu868-stm32wl-task branch checked out:

- rustup update (if needed)
- cargo clean
- cargo update
- plug in the stm32wl device
- DEFMT_LOG=info cargo run --release

### Chat

A public chat on LoRa/LoRaWAN topics using Rust is here:

- <a href="https://matrix.to/#/#public-lora-wan-rs:matrix.org">Matrix room</a>
