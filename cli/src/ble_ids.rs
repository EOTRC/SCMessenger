//! BLE wire protocol service and characteristic identity definitions.
//!
//! Shared by the Windows WinRT stack and cross-platform btleplug stack.
//! Changing any UUID without updating all platforms breaks cross-stack discovery.
//! The characteristic UUIDs are the 128-bit expansions of the u16 short forms
//! in `core/src/transport/ble/gatt.rs` (`GattCharacteristic::uuid()`); the two
//! representations must stay in sync.

pub const GATT_SERVICE_UUID: u128 = 0x0000_DF01_0000_1000_8000_0080_5F9B_34FB;
pub const IDENTITY_CHAR_UUID: u128 = 0x0000_DF02_0000_1000_8000_0080_5F9B_34FB;
pub const MESSAGE_CHAR_UUID: u128 = 0x0000_DF03_0000_1000_8000_0080_5F9B_34FB;
