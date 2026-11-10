#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProbeResult {
    pub id: String,
    pub physical_id: Option<String>,
    pub name: String,
    pub percent: Option<u8>,
    pub source: String,
    pub status: String,
    pub priority: i32,
}
