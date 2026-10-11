//! Adapter shared by the three user tool managers.
use super::{ScanReport, Scanner};
use crate::domain::{package::PackageSource, system, tool_managers};
use anyhow::Result;
use std::{future::Future, pin::Pin};

pub struct ToolScanner(pub PackageSource);
impl Scanner for ToolScanner {
    fn source(&self) -> PackageSource {
        self.0
    }
    fn is_available(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move { system::resolve(tool_managers::manager_name(self.0)).is_some() })
    }
    fn scan(&self) -> Pin<Box<dyn Future<Output = Result<ScanReport>> + Send + '_>> {
        Box::pin(tool_managers::scan(self.0))
    }
}
