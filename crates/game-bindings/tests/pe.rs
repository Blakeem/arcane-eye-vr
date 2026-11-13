//! The PE section walk and the signature resolve on a small PE built here and on this test binary.

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use game_bindings::pe::{CodeSection, ImageLayout, PeError, Rva, code_sections};
    use game_bindings::signatures::{DX11_PRESENT_CALL, SignatureError, resolve_all};

    const PE_OFFSET: usize = 0x80;
    const SECTION_TABLE: usize = PE_OFFSET + 24 + OPTIONAL_HEADER_SIZE;
    const OPTIONAL_HEADER_SIZE: usize = 0xF0;
    /// Code, execute and read.
    const TEXT_FLAGS: u32 = 0x6000_0020;
    /// Initialized data, read and write.
    const DATA_FLAGS: u32 = 0xC000_0040;
    const FILE_SIZE: usize = 0x600;
    const TEXT_RAW: usize = 0x400;
    const TEXT_RAW_SIZE: usize = 0x200;
    const TEXT_VIRTUAL_SIZE: usize = 0x100;
    const TEXT_RVA: usize = 0x1000;
    const DATA_RAW: usize = 0x300;
    const DATA_SIZE: usize = 0x80;
    const DATA_RVA: usize = 0x2000;
    const PRESENT_BYTES: [u8; 20] = [
        0x48, 0x8B, 0x01, 0xFF, 0x50, 0x40, 0x8B, 0xF0, 0x8D, 0x88, 0xFB, 0xFF, 0x85, 0x77, 0xF7,
        0xC1, 0xFD, 0xFF, 0xFF, 0xFF,
    ];

    struct SectionSpec {
        name: &'static [u8],
        virtual_size: usize,
        rva: usize,
        raw_size: usize,
        raw: usize,
        flags: u32,
    }

    const SECTIONS: [SectionSpec; 2] = [
        SectionSpec {
            name: b".text",
            virtual_size: TEXT_VIRTUAL_SIZE,
            rva: TEXT_RVA,
            raw_size: TEXT_RAW_SIZE,
            raw: TEXT_RAW,
            flags: TEXT_FLAGS,
        },
        SectionSpec {
            name: b".data",
            virtual_size: DATA_SIZE,
            rva: DATA_RVA,
            raw_size: DATA_SIZE,
            raw: DATA_RAW,
            flags: DATA_FLAGS,
        },
    ];

    fn put(image: &mut [u8], offset: usize, bytes: &[u8]) {
        image[offset..][..bytes.len()].copy_from_slice(bytes);
    }

    fn put_u32(image: &mut [u8], offset: usize, value: usize) {
        put(image, offset, &u32::try_from(value).unwrap().to_le_bytes());
    }

    fn section_header(section: &SectionSpec) -> [u8; 40] {
        let mut header = [0u8; 40];
        put(&mut header, 0, section.name);
        put_u32(&mut header, 8, section.virtual_size);
        put_u32(&mut header, 12, section.rva);
        put_u32(&mut header, 16, section.raw_size);
        put_u32(&mut header, 20, section.raw);
        put(&mut header, 36, &section.flags.to_le_bytes());
        header
    }

    /// A PE file with a `.text` section and a `.data` section. The Present bytes sit in `.text` at
    /// offset 0x20, in `.data`, and in the `.text` raw padding past its virtual size.
    fn pe_file() -> Vec<u8> {
        let mut image = vec![0u8; FILE_SIZE];
        put(&mut image, 0, b"MZ");
        put_u32(&mut image, 0x3C, PE_OFFSET);
        put(&mut image, PE_OFFSET, b"PE\0\0");
        put(&mut image, PE_OFFSET + 4, &0x8664u16.to_le_bytes());
        put(&mut image, PE_OFFSET + 6, &2u16.to_le_bytes());
        put(
            &mut image,
            PE_OFFSET + 20,
            &u16::try_from(OPTIONAL_HEADER_SIZE).unwrap().to_le_bytes(),
        );
        let (table, _) = image[SECTION_TABLE..].as_chunks_mut::<40>();
        for (slot, section) in table.iter_mut().zip(&SECTIONS) {
            *slot = section_header(section);
        }
        put(&mut image, TEXT_RAW + 0x20, &PRESENT_BYTES);
        put(&mut image, DATA_RAW + 0x10, &PRESENT_BYTES);
        put(&mut image, TEXT_RAW + TEXT_VIRTUAL_SIZE + 0x10, &PRESENT_BYTES);
        image
    }

    /// The same PE as the loader maps it: headers at 0, each section at its RVA.
    fn pe_loaded(file: &[u8]) -> Vec<u8> {
        let mut image = vec![0u8; DATA_RVA + DATA_SIZE];
        image[..SECTION_TABLE + 80].copy_from_slice(&file[..SECTION_TABLE + 80]);
        for section in &SECTIONS {
            let raw = &file[section.raw..][..section.raw_size];
            let mapped = section.raw_size.min(section.virtual_size);
            put(&mut image, section.rva, &raw[..mapped]);
        }
        image
    }

    #[test]
    fn file_layout_finds_only_the_executable_section_up_to_its_virtual_size() {
        let file = pe_file();
        let sections = code_sections(&file, ImageLayout::File).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].rva, Rva(0x1000));
        assert_eq!(sections[0].bytes, &file[TEXT_RAW..TEXT_RAW + TEXT_VIRTUAL_SIZE]);
    }

    #[test]
    fn zero_virtual_size_takes_the_raw_size() {
        let mut file = pe_file();
        put_u32(&mut file, SECTION_TABLE + 8, 0);
        let sections = code_sections(&file, ImageLayout::File).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].bytes, &file[TEXT_RAW..TEXT_RAW + TEXT_RAW_SIZE]);
    }

    #[test]
    fn loaded_layout_reads_the_section_at_its_rva() {
        let loaded = pe_loaded(&pe_file());
        let sections = code_sections(&loaded, ImageLayout::Loaded).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].rva, Rva(0x1000));
        assert_eq!(sections[0].bytes, &loaded[TEXT_RVA..TEXT_RVA + TEXT_VIRTUAL_SIZE]);
    }

    #[test]
    fn present_call_resolves_to_the_same_rva_in_both_layouts() {
        let file = pe_file();
        let loaded = pe_loaded(&file);
        for (image, layout) in [(&file, ImageLayout::File), (&loaded, ImageLayout::Loaded)] {
            let resolved = resolve_all(image, layout).unwrap();
            let present = resolved.iter().find(|entry| *entry.signature == DX11_PRESENT_CALL);
            assert_eq!(present.map(|entry| entry.result), Some(Ok(Rva(0x1020))), "{layout:?}");
        }
    }

    #[test]
    fn present_call_twice_in_the_code_is_ambiguous() {
        let mut file = pe_file();
        put(&mut file, TEXT_RAW + 0x80, &PRESENT_BYTES);
        let sections = code_sections(&file, ImageLayout::File).unwrap();
        let result = DX11_PRESENT_CALL.resolve(&sections);
        assert_eq!(result, Err(SignatureError::Ambiguous { matches: 2 }));
    }

    #[test]
    fn broken_headers_are_named_errors() {
        let file = pe_file();
        assert_eq!(code_sections(&file[..1], ImageLayout::File), Err(PeError::NoDosHeader));
        assert_eq!(
            code_sections(&file[..0x3E], ImageLayout::File),
            Err(PeError::Truncated { offset: 0x3C })
        );

        let mut no_signature = file.clone();
        put(&mut no_signature, PE_OFFSET, b"XX");
        assert_eq!(
            code_sections(&no_signature, ImageLayout::File),
            Err(PeError::NoPeSignature { offset: PE_OFFSET })
        );

        let cut = &file[..TEXT_RAW + 0x10];
        assert_eq!(
            code_sections(cut, ImageLayout::File),
            Err(PeError::SectionOutOfBounds {
                index: 0,
                start: TEXT_RAW,
                end: TEXT_RAW + 0x100,
                len: cut.len()
            })
        );

        let mut no_code = file;
        put(&mut no_code, SECTION_TABLE + 36, &DATA_FLAGS.to_le_bytes());
        assert_eq!(code_sections(&no_code, ImageLayout::File), Err(PeError::NoCodeSection));
    }

    #[test]
    fn walk_of_this_test_binary_finds_its_code() {
        let exe = fs::read(env::current_exe().unwrap()).unwrap();
        let sections = code_sections(&exe, ImageLayout::File).unwrap();
        let total: usize =
            sections.iter().map(|section: &CodeSection<'_>| section.bytes.len()).sum();
        // The headers take the first page, and this binary holds at least this test's code.
        assert!(sections.iter().all(|section| section.rva >= Rva(0x1000)));
        assert!(total > 4096, "code bytes {total}");
    }
}
