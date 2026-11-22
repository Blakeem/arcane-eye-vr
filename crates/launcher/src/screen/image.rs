//! The target of an acquired swapchain image, which each draw into a swapchain of the headset
//! session shares: the screen's eyes, the panels and the controller models' layer.

/// The target of the swapchain image `index` among `targets`, one per image of the swapchain, or
/// `None` for an index past `targets`.
pub(super) fn target<T>(targets: &[T], index: u32) -> Option<&T> {
    usize::try_from(index).ok().and_then(|slot| targets.get(slot))
}

#[cfg(test)]
mod tests {
    use super::target;

    #[test]
    fn an_image_index_names_its_target_and_an_index_past_the_targets_names_none() {
        let targets = ["first", "second"];
        assert_eq!(target(&targets, 0), Some(&"first"));
        assert_eq!(target(&targets, 1), Some(&"second"));
        assert_eq!(target(&targets, 2), None);
        assert_eq!(target(&targets, u32::MAX), None);
    }
}
