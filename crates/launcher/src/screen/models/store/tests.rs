use super::Store;

fn kept(store: &Store<u32, &'static str>, keys: &[u32]) -> Vec<&'static str> {
    let tracked = ();
    let frames = keys.iter().map(|&key| (key, Some(&tracked)));
    store.tracked(frames).map(|(model, ())| *model).collect()
}

#[test]
fn a_model_that_left_is_released_and_the_others_stay() {
    let mut store = Store::new();
    store.apply(&[], [(1, "left hand"), (2, "right hand")]);
    store.apply(&[1], []);
    assert_eq!(kept(&store, &[1, 2]), ["right hand"]);
}

#[test]
fn a_key_that_left_and_arrived_again_keeps_its_new_model() {
    let mut store = Store::new();
    store.apply(&[], [(1, "old")]);
    store.apply(&[1], [(1, "new")]);
    assert_eq!(kept(&store, &[1]), ["new"]);
    store.apply(&[], [(1, "newer")]);
    assert_eq!(kept(&store, &[1]), ["newer"], "an arrival replaces the model of its key");
}

#[test]
fn an_untracked_frame_or_a_frame_of_an_unknown_key_draws_nothing() {
    let mut store = Store::new();
    store.apply(&[], [(1, "left hand")]);
    let untracked: [(u32, Option<&()>); 1] = [(1, None)];
    assert_eq!(store.tracked(untracked).count(), 0, "an untracked model");
    assert_eq!(kept(&store, &[7]), Vec::<&str>::new(), "a key the store does not keep");
}
