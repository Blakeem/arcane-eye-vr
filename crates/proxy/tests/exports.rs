//! The built version.dll exports the 17 version.dll names, each as a forwarder to the System32 copy.
//!
//! The test reads the export directory from the DLL file, so it checks the strings the loader reads.

#[cfg(test)]
mod tests {
    include!("../forwarders.rs");

    use std::ffi::CStr;
    use std::{env, fs};

    /// The DOS header field that holds the file offset of the PE signature.
    const PE_HEADER_POINTER: usize = 0x3C;
    const PE_SIGNATURE: &[u8] = b"PE\0\0";

    const COFF_SECTION_COUNT: usize = 2;
    const COFF_OPTIONAL_SIZE: usize = 16;
    const COFF_HEADER_SIZE: usize = 20;

    /// The PE32+ optional header magic.
    const PE32_PLUS: u16 = 0x20B;
    /// The PE32+ optional header offset of the export data directory's address. Its size follows.
    const OPTIONAL_EXPORT_RVA: usize = 112;
    const OPTIONAL_EXPORT_SIZE: usize = 116;

    const SECTION_VIRTUAL_SIZE: usize = 8;
    const SECTION_VIRTUAL_ADDRESS: usize = 12;
    const SECTION_RAW_OFFSET: usize = 20;
    const SECTION_HEADER_SIZE: usize = 40;

    const EXPORT_NAME_COUNT: usize = 24;
    const EXPORT_FUNCTIONS: usize = 28;
    const EXPORT_NAMES: usize = 32;
    const EXPORT_ORDINALS: usize = 36;

    /// The entry size of the export address and name tables.
    const RVA_ENTRY_SIZE: usize = 4;
    const ORDINAL_ENTRY_SIZE: usize = 2;

    fn u16_at(bytes: &[u8], offset: usize) -> u16 {
        let field = bytes.get(offset..).and_then(|rest| rest.first_chunk::<2>());
        u16::from_le_bytes(*field.expect("a u16 inside the file"))
    }

    fn u32_at(bytes: &[u8], offset: usize) -> usize {
        let field = bytes.get(offset..).and_then(|rest| rest.first_chunk::<4>());
        u32::from_le_bytes(*field.expect("a u32 inside the file")) as usize
    }

    fn add(base: usize, offset: usize) -> usize {
        base.checked_add(offset).expect("an offset inside the file")
    }

    /// The offset of entry `index` in a table of `entry_size`-byte entries at `table`.
    fn entry(table: usize, index: usize, entry_size: usize) -> usize {
        add(table, index.checked_mul(entry_size).expect("an entry inside the file"))
    }

    fn c_string_at(bytes: &[u8], offset: usize) -> String {
        let rest = bytes.get(offset..).expect("a string inside the file");
        let text = CStr::from_bytes_until_nul(rest).expect("a NUL-terminated string");
        text.to_str().expect("an ASCII string").to_owned()
    }

    /// A section's virtual address range and where its bytes start in the file.
    struct Section {
        virtual_address: usize,
        virtual_size: usize,
        raw_offset: usize,
    }

    /// The PE image of the built proxy, read from its file.
    struct Image {
        bytes: Vec<u8>,
        sections: Vec<Section>,
        export_rva: usize,
        export_size: usize,
    }

    impl Image {
        fn read() -> Self {
            let exe = env::current_exe().expect("the test binary path");
            let path = exe.with_file_name(format!("{DLL_NAME}.dll"));
            let bytes = fs::read(&path).expect("the built proxy next to the test binary");
            let pe_header = u32_at(&bytes, PE_HEADER_POINTER);
            let coff = add(pe_header, PE_SIGNATURE.len());
            assert_eq!(bytes.get(pe_header..coff), Some(PE_SIGNATURE));
            let section_count = usize::from(u16_at(&bytes, add(coff, COFF_SECTION_COUNT)));
            let optional_size = usize::from(u16_at(&bytes, add(coff, COFF_OPTIONAL_SIZE)));
            let optional = add(coff, COFF_HEADER_SIZE);
            assert_eq!(u16_at(&bytes, optional), PE32_PLUS);
            let export_rva = u32_at(&bytes, add(optional, OPTIONAL_EXPORT_RVA));
            let export_size = u32_at(&bytes, add(optional, OPTIONAL_EXPORT_SIZE));
            let section_table = add(optional, optional_size);
            let sections = (0..section_count)
                .map(|index| {
                    let header = entry(section_table, index, SECTION_HEADER_SIZE);
                    Section {
                        virtual_size: u32_at(&bytes, add(header, SECTION_VIRTUAL_SIZE)),
                        virtual_address: u32_at(&bytes, add(header, SECTION_VIRTUAL_ADDRESS)),
                        raw_offset: u32_at(&bytes, add(header, SECTION_RAW_OFFSET)),
                    }
                })
                .collect();
            Self { bytes, sections, export_rva, export_size }
        }

        fn file_offset(&self, rva: usize) -> usize {
            let section = self
                .sections
                .iter()
                .find(|section| {
                    rva >= section.virtual_address
                        && rva < add(section.virtual_address, section.virtual_size)
                })
                .expect("an address inside a section");
            let into_section = rva.checked_sub(section.virtual_address).expect("found above");
            add(section.raw_offset, into_section)
        }

        fn u32_at_rva(&self, rva: usize) -> usize {
            u32_at(&self.bytes, self.file_offset(rva))
        }

        /// Every named export with its forwarder string, or `None` for an export with code.
        fn exports(&self) -> Vec<(String, Option<String>)> {
            let directory = self.export_rva;
            let functions = self.u32_at_rva(add(directory, EXPORT_FUNCTIONS));
            let names = self.u32_at_rva(add(directory, EXPORT_NAMES));
            let ordinals = self.u32_at_rva(add(directory, EXPORT_ORDINALS));
            (0..self.u32_at_rva(add(directory, EXPORT_NAME_COUNT)))
                .map(|index| {
                    let name_rva = self.u32_at_rva(entry(names, index, RVA_ENTRY_SIZE));
                    let ordinal_offset =
                        self.file_offset(entry(ordinals, index, ORDINAL_ENTRY_SIZE));
                    let ordinal = usize::from(u16_at(&self.bytes, ordinal_offset));
                    let target = self.u32_at_rva(entry(functions, ordinal, RVA_ENTRY_SIZE));
                    let forwards = target >= directory && target < add(directory, self.export_size);
                    let forwarder =
                        forwards.then(|| c_string_at(&self.bytes, self.file_offset(target)));
                    (c_string_at(&self.bytes, self.file_offset(name_rva)), forwarder)
                })
                .collect()
        }
    }

    #[test]
    fn each_version_export_forwards_to_the_system32_copy() {
        let exports = Image::read().exports();
        for (name, forwarder) in forwarders() {
            let found = exports.iter().find(|(export, _)| export == name);
            let (_, found_forwarder) = found.unwrap_or_else(|| panic!("{name} is not exported"));
            assert_eq!(found_forwarder.as_deref(), Some(forwarder.as_str()), "{name}");
        }
        let expected = r"\\.\GLOBALROOT\SystemRoot\System32\version.VerQueryValueW".to_owned();
        assert!(exports.contains(&("VerQueryValueW".to_owned(), Some(expected))));
    }

    #[test]
    fn no_other_export_forwards() {
        let exports = Image::read().exports();
        let forwarding = exports.iter().filter(|(_, forwarder)| forwarder.is_some()).count();
        assert_eq!(forwarding, 17);
    }
}
