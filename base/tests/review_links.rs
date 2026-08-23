use ironcalc_base::{ChangedSinceRead, Model, RecalcMode};
fn delta(x: &mut Model) -> String {
    match x.take_changed_cells() {
        ChangedSinceRead::Everything => "Everything".into(),
        ChangedSinceRead::Cells(c) => format!(
            "{:?}",
            c.iter().map(|p| (p.row, p.column)).collect::<Vec<_>>()
        ),
    }
}
#[test]
fn link_edits_are_journaled() {
    let mut x = Model::new_empty("t", "en", "UTC", "en")
        .unwrap()
        .with_recalc_mode(RecalcMode::Incremental);
    x.set_user_input(0, 1, 1, "hello".to_string()).unwrap();
    x.evaluate();
    let _ = x.take_changed_cells();
    x.set_cell_link(
        0,
        1,
        1,
        ironcalc_base::types::Link::External {
            target: "https://ironcalc.com".to_string(),
            tooltip: None,
        },
    )
    .unwrap();
    x.evaluate();
    let d1 = delta(&mut x);
    println!("after set_cell_link: delta={d1}");
    x.delete_cell_link(0, 1, 1).unwrap();
    x.evaluate();
    let d2 = delta(&mut x);
    println!("after delete_cell_link: delta={d2}");
    assert!(
        d1.contains("(1, 1)") || d1 == "Everything",
        "set_cell_link not in delta"
    );
    assert!(
        d2.contains("(1, 1)") || d2 == "Everything",
        "delete_cell_link not in delta"
    );
}

#[test]
fn range_clear_journals_stranded_links() {
    // A link can sit at a position with no cell (structural edits strand
    // them). Clearing a range that covers it must journal the link removal,
    // or the delta misses the change (fuzz seed 18 at 200 steps).
    let mut x = Model::new_empty("t", "en", "UTC", "en")
        .unwrap()
        .with_recalc_mode(RecalcMode::Incremental);
    x.set_cell_link(
        0,
        2,
        2,
        ironcalc_base::types::Link::External {
            target: "https://ironcalc.com".to_string(),
            tooltip: None,
        },
    )
    .unwrap();
    x.evaluate();
    let _ = x.take_changed_cells();
    assert_eq!(x.get_links_list(0).unwrap().len(), 1);
    x.range_clear_contents(&ironcalc_base::expressions::types::Area {
        sheet: 0,
        row: 1,
        column: 1,
        width: 3,
        height: 3,
    })
    .unwrap();
    x.evaluate();
    assert_eq!(x.get_links_list(0).unwrap().len(), 0, "link not removed");
    let d = delta(&mut x);
    println!("after range_clear_contents: delta={d}");
    assert!(
        d.contains("(2, 2)") || d == "Everything",
        "stranded link removal not in delta: {d}"
    );
}
