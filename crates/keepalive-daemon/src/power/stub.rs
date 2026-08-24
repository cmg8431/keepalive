/// No-op assertion for non-macOS targets so the workspace type-checks anywhere.
pub struct WakeAssertion;

impl WakeAssertion {
    pub fn new(_reason: &str) -> Option<Self> {
        Some(Self)
    }
}

pub struct SmcReader;

impl SmcReader {
    pub fn new() -> Option<Self> {
        None
    }

    pub fn temperature(&self) -> Option<f64> {
        None
    }
}
