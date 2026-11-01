use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
pub struct HwParams {
    pub path: String,
    pub format: String,
    pub rate: u32,
    pub channels: Option<u32>,
    pub period_size: Option<u32>,
    pub buffer_size: Option<u32>,
    pub bits: Option<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct Sink {
    pub id: String,
    pub description: String,
}
