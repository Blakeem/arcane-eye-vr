//! The swapchain format and the view a draw writes through.
//!
//! The game's published values are the values the monitor shows, so they are sRGB-encoded. The
//! runtime reads a swapchain of an sRGB format as sRGB-encoded values, so the headset shows the
//! stored values as the monitor does. A D3D11 swapchain image is typeless, so a draw writes it
//! through a view of the plain format when it writes the source's stored values, and through a view
//! of the sRGB format when its source read decoded them from sRGB.

use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_FORMAT_B8G8R8X8_UNORM_SRGB, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
};

/// What a shader read of a source texture returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sampling {
    /// The stored values, as for a texture of a plain format.
    Stored,
    /// The stored values decoded from sRGB, as for a texture of an sRGB format.
    SrgbDecoded,
}

impl Sampling {
    /// What a read of a texture of `format` through a view of its own format returns.
    pub const fn of(format: DXGI_FORMAT) -> Self {
        match format {
            DXGI_FORMAT_R8G8B8A8_UNORM_SRGB
            | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
            | DXGI_FORMAT_B8G8R8X8_UNORM_SRGB => Self::SrgbDecoded,
            _ => Self::Stored,
        }
    }
}

/// An 8-bit sRGB swapchain format and the plain format of the same layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwapchainFormat {
    srgb: DXGI_FORMAT,
    plain: DXGI_FORMAT,
}

/// The formats the screen's swapchains take.
const CHOICES: [SwapchainFormat; 2] = [
    SwapchainFormat { srgb: DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, plain: DXGI_FORMAT_R8G8B8A8_UNORM },
    SwapchainFormat { srgb: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, plain: DXGI_FORMAT_B8G8R8A8_UNORM },
];

impl SwapchainFormat {
    /// The format the swapchain is created with.
    pub const fn swapchain(self) -> DXGI_FORMAT {
        self.srgb
    }

    /// The view format that stores the values a draw's source holds when the draw reads the
    /// source as `sampling` says.
    pub const fn target_view(self, sampling: Sampling) -> DXGI_FORMAT {
        match sampling {
            Sampling::Stored => self.plain,
            Sampling::SrgbDecoded => self.srgb,
        }
    }
}

/// The first format of `offered`, which lists the runtime's formats from its most preferred, that
/// the screen takes. Returns `None` when the runtime offers no 8-bit sRGB format.
pub fn choose(offered: &[u32]) -> Option<SwapchainFormat> {
    offered.iter().find_map(|&format| {
        CHOICES.into_iter().find(|choice| u32::try_from(choice.srgb.0) == Ok(format))
    })
}

#[cfg(test)]
mod tests {
    use super::{CHOICES, Sampling, choose};
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_R8G8B8A8_UNORM,
        DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_R10G10B10A2_UNORM,
    };

    /// The formats SteamVR 2.18.4 offered, in its order.
    const STEAMVR_FORMATS: [u32; 5] = [29, 91, 2, 10, 24];

    #[test]
    fn the_runtime_s_most_preferred_srgb_format_is_taken() {
        let chosen = choose(&STEAMVR_FORMATS).expect("an sRGB format");
        assert_eq!(chosen.swapchain(), DXGI_FORMAT_R8G8B8A8_UNORM_SRGB);
        let bgra_first = choose(&[2, 91, 29]).expect("an sRGB format");
        assert_eq!(bgra_first.swapchain(), DXGI_FORMAT_B8G8R8A8_UNORM_SRGB);
    }

    #[test]
    fn a_runtime_without_an_8_bit_srgb_format_gives_no_format() {
        assert_eq!(choose(&[2, 10, 24, 28, 87]), None);
        assert_eq!(choose(&[]), None);
    }

    #[test]
    fn stored_values_are_written_through_the_plain_view_and_decoded_values_through_the_srgb_view() {
        let [rgba, bgra] = CHOICES;
        assert_eq!(rgba.target_view(Sampling::Stored), DXGI_FORMAT_R8G8B8A8_UNORM);
        assert_eq!(rgba.target_view(Sampling::SrgbDecoded), DXGI_FORMAT_R8G8B8A8_UNORM_SRGB);
        assert_eq!(bgra.target_view(Sampling::Stored), DXGI_FORMAT_B8G8R8A8_UNORM);
        assert_eq!(bgra.target_view(Sampling::SrgbDecoded), DXGI_FORMAT_B8G8R8A8_UNORM_SRGB);
    }

    #[test]
    fn only_srgb_sources_are_read_decoded() {
        assert_eq!(Sampling::of(DXGI_FORMAT_R10G10B10A2_UNORM), Sampling::Stored);
        assert_eq!(Sampling::of(DXGI_FORMAT_R8G8B8A8_UNORM), Sampling::Stored);
        assert_eq!(Sampling::of(DXGI_FORMAT_B8G8R8A8_UNORM), Sampling::Stored);
        assert_eq!(Sampling::of(DXGI_FORMAT_R8G8B8A8_UNORM_SRGB), Sampling::SrgbDecoded);
        assert_eq!(Sampling::of(DXGI_FORMAT_B8G8R8A8_UNORM_SRGB), Sampling::SrgbDecoded);
    }
}
