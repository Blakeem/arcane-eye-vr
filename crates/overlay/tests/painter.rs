//! The painter's pixels for a menu, a point and a press, and when it reports a change.

#[cfg(test)]
mod tests {
    use gfx::layer::Rect;
    use overlay::layout::{PanelPoint, layout};
    use overlay::menu::{ButtonState, Menu, Row};
    use overlay::painter::{Change, Painter, Press};

    const ROW_BYTES: usize = 640 * 4;
    /// The center of row 0's plus button, which spans x 584 to 624 and y 60 to 100.
    const OVER_PLUS: PanelPoint = PanelPoint { x: 604.0, y: 80.0 };

    fn menu(distance: &str) -> Menu {
        let row = |label: &str, value: &str| Row {
            label: label.to_owned(),
            value: value.to_owned(),
            minus: ButtonState::Enabled,
            plus: ButtonState::Enabled,
        };
        let rows = vec![row("Distance", distance), row("Size", "1.6 m")];
        Menu::new("Screen".to_owned(), rows).expect("two rows")
    }

    /// The pixels of `rect` in `pixels`, a picture 640 pixels wide, rows top first.
    fn area(pixels: &[u8], rect: Rect) -> Vec<[u8; 4]> {
        let columns = usize::from(rect.x)..usize::from(rect.x.saturating_add(rect.width));
        let rows =
            pixels.chunks(ROW_BYTES).skip(usize::from(rect.y)).take(usize::from(rect.height));
        rows.flat_map(|row| row.as_chunks::<4>().0[columns.clone()].to_vec()).collect()
    }

    fn paint(
        painter: &mut Painter,
        menu: &Menu,
        point: Option<PanelPoint>,
        press: Press,
    ) -> Vec<u8> {
        painter.paint(menu, point, press).expect("a paint").pixels.to_vec()
    }

    #[test]
    fn the_pixels_fill_the_panel_and_every_alpha_is_255() {
        let mut painter = Painter::new().expect("a painter");

        let pixels = paint(&mut painter, &menu("2.5 m"), None, Press::Released);

        assert_eq!(pixels.len(), 640 * 360 * 4);
        assert!(pixels.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn the_button_under_the_point_changes_its_pixels_and_changes_them_again_when_pressed() {
        let mut painter = Painter::new().expect("a painter");
        let menu = menu("2.5 m");
        let plus = layout(&menu).rows[0].plus.rect;

        let idle = area(&paint(&mut painter, &menu, None, Press::Released), plus);
        let hover = area(&paint(&mut painter, &menu, Some(OVER_PLUS), Press::Released), plus);
        let pressed = area(&paint(&mut painter, &menu, Some(OVER_PLUS), Press::Pressed), plus);

        assert_ne!(idle, hover);
        assert_ne!(hover, pressed);
        assert_ne!(idle, pressed);
    }

    #[test]
    fn a_row_label_draws_pixels_that_differ_from_the_background() {
        let mut painter = Painter::new().expect("a painter");
        let menu = menu("2.5 m");
        let label = layout(&menu).rows[0].label;

        let pixels = paint(&mut painter, &menu, None, Press::Released);

        // The bottom right corner lies below the last row, so it shows the background.
        let background = area(&pixels, Rect { x: 639, y: 359, width: 1, height: 1 })[0];
        assert!(area(&pixels, label).iter().any(|&pixel| pixel != background));
    }

    #[test]
    fn the_same_inputs_report_no_change_and_another_value_reports_a_change() {
        let mut painter = Painter::new().expect("a painter");

        let first = painter.paint(&menu("2.5 m"), None, Press::Released).expect("a paint").change;
        let same = painter.paint(&menu("2.5 m"), None, Press::Released).expect("a paint").change;
        let other = painter.paint(&menu("2.6 m"), None, Press::Released).expect("a paint").change;

        assert_eq!((first, same, other), (Change::Redrawn, Change::Unchanged, Change::Redrawn));
    }

    #[test]
    fn a_point_that_moves_inside_one_button_or_a_press_over_no_button_reports_no_change() {
        let mut painter = Painter::new().expect("a painter");
        let menu = menu("2.5 m");
        let moved = PanelPoint { x: 590.5, y: 95.25 };
        let on_title = PanelPoint { x: 100.0, y: 30.0 };

        painter.paint(&menu, Some(OVER_PLUS), Press::Released).expect("a paint");
        let inside = painter.paint(&menu, Some(moved), Press::Released).expect("a paint").change;
        painter.paint(&menu, None, Press::Released).expect("a paint");
        let off = painter.paint(&menu, Some(on_title), Press::Pressed).expect("a paint").change;

        assert_eq!((inside, off), (Change::Unchanged, Change::Unchanged));
    }

    #[test]
    fn an_unchanged_paint_returns_the_pixels_of_the_last_paint() {
        let mut painter = Painter::new().expect("a painter");
        let menu = menu("2.5 m");

        let first = paint(&mut painter, &menu, Some(OVER_PLUS), Press::Released);
        let again = paint(&mut painter, &menu, Some(OVER_PLUS), Press::Released);

        assert_eq!(first, again);
    }
}
