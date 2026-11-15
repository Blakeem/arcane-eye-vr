//! The game exe's module in memory: its load address and the address ranges of its executable code.
//! Also a loaded module by its name, and whether an address lies in it.
//!
//! The ranges come from the memory manager's view of the module's pages, not from its PE headers, so
//! they hold exactly the pages a guarded read of code can copy.

use crate::GameAddr;
use crate::raw::{self, Access, Region};

/// A run of executable pages in the main module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeRange {
    pub start: GameAddr,
    pub len: usize,
}

/// The process exe's module: its load address and its executable ranges in address order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MainModule {
    pub base: GameAddr,
    pub code: Vec<CodeRange>,
}

/// Why the main module's code ranges are unknown.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ModuleError {
    #[error("Windows reports no module for the process exe: {0}")]
    NoModule(#[from] windows::core::Error),
    #[error("the module at {base:#x} has no executable pages")]
    NoCode { base: usize },
}

/// The process exe's module and its executable ranges.
///
/// # Errors
///
/// Returns [`ModuleError::NoModule`] when Windows reports no exe module, and [`ModuleError::NoCode`]
/// when no page of it is executable.
pub fn main_module() -> Result<MainModule, ModuleError> {
    let base = raw::main_module_base()?;
    let mut code = Vec::new();
    let mut cursor = Some(base);
    while let Some(region) = cursor.and_then(raw::query) {
        if region.allocation_base != base {
            break;
        }
        if let Some(range) = code_range(&region) {
            push_merged(&mut code, range);
        }
        cursor = region.end();
    }
    if code.is_empty() {
        return Err(ModuleError::NoCode { base });
    }
    Ok(MainModule { base: GameAddr::new(base), code })
}

/// A module the process has loaded, by its load address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadedModule(GameAddr);

impl LoadedModule {
    /// The loaded module named `name`, such as `d3d11.dll`, or `None` when the process holds none.
    pub fn named(name: &str) -> Option<Self> {
        raw::module_base(name).map(|base| Self(GameAddr::new(base)))
    }

    pub const fn base(self) -> GameAddr {
        self.0
    }

    /// Whether `address` lies in the pages Windows mapped for the module's image.
    pub fn holds(self, address: GameAddr) -> bool {
        raw::query(address.get()).is_some_and(|region| region.allocation_base == self.0.get())
    }
}

fn code_range(region: &Region) -> Option<CodeRange> {
    match region.access {
        Access::Code => Some(CodeRange { start: GameAddr::new(region.base), len: region.size }),
        Access::Other => None,
    }
}

/// Appends `range`, or extends the last range when `range` starts where it ends. `VirtualQuery` splits
/// executable pages wherever their protection differs, and a pattern can cross that split.
fn push_merged(code: &mut Vec<CodeRange>, range: CodeRange) {
    if let Some(last) = code.last_mut()
        && last.start.checked_add(last.len) == Some(range.start)
        && let Some(len) = last.len.checked_add(range.len)
    {
        last.len = len;
        return;
    }
    code.push(range);
}

#[cfg(test)]
mod tests {
    use super::{CodeRange, LoadedModule, main_module, push_merged};
    use crate::GameAddr;

    #[test]
    fn a_loaded_module_holds_its_own_pages_and_no_other_address() {
        let kernel = LoadedModule::named("kernel32.dll").expect("every process loads kernel32");
        assert!(kernel.holds(kernel.base()));
        assert!(kernel.holds(kernel.base().checked_add(0x1000).expect("a test address")));
        let exe = main_module().expect("the test exe's module");
        assert!(!kernel.holds(exe.base), "the exe is another module");
        let local = 0_u8;
        assert!(!kernel.holds(GameAddr::new((&raw const local).expose_provenance())));
        assert!(!kernel.holds(GameAddr::new(0x10)), "an unmapped address");
        assert_eq!(LoadedModule::named("arcane-eye-no-such-module.dll"), None);
    }

    fn range(start: usize, len: usize) -> CodeRange {
        CodeRange { start: GameAddr::new(start), len }
    }

    #[test]
    fn push_merged_joins_adjacent_ranges_and_keeps_a_gap() {
        let mut code = Vec::new();
        push_merged(&mut code, range(0x1000, 0x1000));
        push_merged(&mut code, range(0x2000, 0x3000));
        push_merged(&mut code, range(0x6000, 0x1000));
        assert_eq!(code, [range(0x1000, 0x4000), range(0x6000, 0x1000)]);
    }
}
