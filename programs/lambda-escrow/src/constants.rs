use anchor_lang::prelude::*;

#[constant]
pub const DISPUTE_WINDOW_SECONDS: i64 = 86_400; // 24 hours

#[constant]
pub const TRACKING_ID_MIN_LEN: u32 = 8;

#[constant]
pub const TRACKING_ID_MAX_LEN: u32 = 50;

#[constant]
pub const MAX_CARRIER_CODE: u8 = 3;

#[constant]
pub const PLATFORM_FEE_PERCENTAGE: u32 = 5;
