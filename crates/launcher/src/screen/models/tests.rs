use openxr::Posef;
use xr::model::TrackedModel;

use super::layer_size;
use super::load::{Arrived, read_arrived};
use super::store::Store;

/// A GLB of one JSON chunk: a default scene of one node "body" with no mesh.
fn glb() -> Vec<u8> {
    let mut json =
        br#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"name":"body"}]}"#
            .to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let json_length = u32::try_from(json.len()).expect("a short chunk");
    let total = json_length.checked_add(20).expect("a short file");
    let mut file = Vec::new();
    let (magic, json_chunk) = (u32::from_le_bytes(*b"glTF"), u32::from_le_bytes(*b"JSON"));
    for word in [magic, 2, total, json_length, json_chunk] {
        file.extend_from_slice(&u32::to_le_bytes(word));
    }
    file.extend_from_slice(&json);
    file
}

#[test]
fn the_layer_holds_each_side_of_the_recommended_size_to_2048_on_its_own() {
    assert_eq!(layer_size((2460, 2590)), (2048, 2048));
    assert_eq!(layer_size((1832, 1920)), (1832, 1920));
    assert_eq!(layer_size((2460, 1920)), (2048, 1920));
}

#[test]
fn a_model_whose_glb_fails_to_read_is_never_drawn_and_the_other_model_is() {
    let names = ["body".to_owned()];
    let (unreadable, readable) = (b"no GLB".to_vec(), glb());
    let arrived = [
        (1_u64, unreadable.as_slice(), names.as_slice()),
        (2, readable.as_slice(), names.as_slice()),
    ];
    let Arrived { read, unread } = read_arrived(arrived);
    let unread_keys: Vec<u64> = unread.iter().map(|(key, _)| *key).collect();
    assert_eq!(unread_keys, [1], "only the GLB that is no GLB is refused");
    let mut store = Store::new();
    store.apply(&[], read.into_iter().map(|(key, read)| (key, read.skeleton)));
    let tracked = TrackedModel { pose: Posef::IDENTITY, nodes: Vec::new() };
    assert_eq!(store.tracked([(1, Some(&tracked))]).count(), 0, "the unread model is not drawn");
    let drawn: Vec<_> = store.tracked([(2, Some(&tracked))]).collect();
    let [(skeleton, _)] = drawn.as_slice() else {
        panic!("the read model is drawn");
    };
    assert_eq!(skeleton.drawn(Posef::IDENTITY, &[]), [], "the read model's node has no mesh");
    assert_eq!(skeleton.world(Posef::IDENTITY, &[]).len(), 1, "the read model's one node");
}
