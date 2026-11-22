//! The kept render models by their keys: a model that left is released, and only a kept model of a
//! tracked frame is drawn.

/// The models the layer keeps, each under the key the session gave it.
pub(crate) struct Store<K, T> {
    kept: Vec<(K, T)>,
}

impl<K: Copy + PartialEq, T> Store<K, T> {
    pub(crate) const fn new() -> Self {
        Self { kept: Vec::new() }
    }

    /// Releases each model of a key in `left`, then keeps each model of `arrived`. A key that left
    /// and arrived again keeps its new model.
    pub(crate) fn apply(&mut self, left: &[K], arrived: impl IntoIterator<Item = (K, T)>) {
        self.kept.retain(|(key, _)| !left.contains(key));
        for (key, model) in arrived {
            self.kept.retain(|(kept, _)| *kept != key);
            self.kept.push((key, model));
        }
    }

    /// The kept model of each frame of `frames` that is tracked, each frame a key and its tracked
    /// state, with that state. A frame of a key the store does not keep, such as the key of a model
    /// that failed to load, gives nothing.
    pub(crate) fn tracked<'a, S: 'a>(
        &'a self,
        frames: impl IntoIterator<Item = (K, Option<&'a S>)>,
    ) -> impl Iterator<Item = (&'a T, &'a S)> {
        frames.into_iter().filter_map(|(key, tracked)| {
            let tracked = tracked?;
            let (_, model) = self.kept.iter().find(|(kept, _)| *kept == key)?;
            Some((model, tracked))
        })
    }
}

#[cfg(test)]
mod tests;
