use super::*;

fn draw(src: &str) -> Diagram {
    render(src, &mut Approx).unwrap()
}

fn texts(d: &Diagram) -> Vec<&str> {
    d.items
        .iter()
        .filter_map(|i| match i {
            Item::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn text_pos(d: &Diagram, t: &str) -> (f32, f32) {
    d.items
        .iter()
        .find_map(|i| match i {
            Item::Text { text, x, y, .. } if text == t => Some((*x, *y)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no text {t}"))
}

#[test]
fn top_down_layers_stack_vertically() {
    let d =
        draw("graph TD\n  A[Start] --> B(Next)\n  B --> C{Choice}\n  C -->|yes| D((Done))\n  C -->|no| A");
    for t in ["Start", "Next", "Choice", "Done", "yes", "no"] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    let (a, b, c) = (text_pos(&d, "Start"), text_pos(&d, "Next"), text_pos(&d, "Choice"));
    assert!(a.1 < b.1 && b.1 < c.1);
    assert!(d.items.iter().any(|i| matches!(i, Item::Diamond { .. })));
    assert!(d.items.iter().any(|i| matches!(i, Item::Ellipse { .. })));
    assert!(d.items.iter().filter(|i| matches!(i, Item::Arrow { .. })).count() >= 4);
    assert!(d.width > 0.0 && d.height > 0.0);
}

#[test]
fn left_right_flows_horizontally() {
    let d = draw("flowchart LR\n A --> B --> C");
    let (a, c) = (text_pos(&d, "A"), text_pos(&d, "C"));
    assert!(a.0 < c.0 && (a.1 - c.1).abs() < 1.0);
    let d = draw("flowchart RL\n A --> B");
    assert!(text_pos(&d, "A").0 > text_pos(&d, "B").0);
    let d = draw("flowchart BT\n A --> B");
    assert!(text_pos(&d, "A").1 > text_pos(&d, "B").1);
}

#[test]
fn edge_syntaxes() {
    let d = draw("graph LR\n A -- text --> B\n B -.-> C\n C ==> D\n D --- E\n E & F --> G\n G <--> H");
    assert!(texts(&d).contains(&"text"));
    let dashed = d.items.iter().any(|i| matches!(i, Item::Line { dashed: true, .. }));
    let thick = d.items.iter().any(|i| matches!(i, Item::Line { width, .. } if *width > 2.0));
    assert!(dashed && thick);
    assert!(texts(&d).contains(&"H"));
}

#[test]
fn cycles_and_self_loops_terminate() {
    let d = draw("graph TD\n A --> B\n B --> C\n C --> A\n A --> A\n A --> C");
    assert!(d.width.is_finite() && d.height.is_finite());
}

#[test]
fn long_edges_get_routed_around() {
    let d = draw("graph TD\n A --> B --> C --> D\n A --> D");
    let routed = d.items.iter().any(|i| matches!(i, Item::Line { pts, .. } if pts.len() > 2));
    assert!(routed);
}

#[test]
fn subgraphs_and_styles_are_tolerated() {
    let d = draw("graph TD\n subgraph one [First]\n A --> B\n end\n classDef x fill:#f9f\n class A x\n style B fill:#333\n B --> C");
    assert!(texts(&d).contains(&"First"));
}

#[test]
fn multiline_and_quoted_labels() {
    let d = draw("graph TD\n A[\"Hello (world)\"] --> B[line one<br/>line two]");
    assert!(texts(&d).contains(&"Hello (world)"));
    assert!(texts(&d).contains(&"line one") && texts(&d).contains(&"line two"));
}

#[test]
fn sequence_diagram() {
    let d = draw("sequenceDiagram\n participant A as Alice\n actor B\n A->>B: Hello\n B-->>A: Hi back\n Note over A,B: thinking\n loop every day\n A->>A: self\n end");
    for t in ["Alice", "B", "Hello", "Hi back", "thinking", "self"] {
        assert!(texts(&d).contains(&t), "{t}");
    }
    let (h, r) = (text_pos(&d, "Hello"), text_pos(&d, "Hi back"));
    assert!(h.1 < r.1);
    assert!(d.items.iter().any(|i| matches!(i, Item::Line { dashed: true, .. })));
}

#[test]
fn unsupported_and_invalid_are_reported() {
    assert_eq!(render("pie title x\n \"a\": 1", &mut Approx).unwrap_err(), Error::Unsupported("pie".into()));
    assert!(matches!(render("graph TD\n A --> ", &mut Approx), Err(Error::Syntax(_))));
    assert!(render("", &mut Approx).is_err());
    assert!(matches!(render("graph XX\n A-->B", &mut Approx), Err(Error::Syntax(_))));
}

#[test]
fn frontmatter_and_comments_are_skipped() {
    let d = draw("---\ntitle: t\n---\n%% note\ngraph TD\n A --> B");
    assert!(texts(&d).contains(&"B"));
}
