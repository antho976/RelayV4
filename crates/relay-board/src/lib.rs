//! The native board's pure logic, kept out of the GTK crate so a headless `cargo test` (and CI)
//! runs it: which tasks a filter and search keep, which group a card sits in and how groups
//! sort, the order a lane shows its cards in, and the final index a drop or a Shift+J/K nudge
//! hands `task.move`. Every function reads `task.list` rows as JSON, exactly as the board does.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Least urgent first: the bus's priority values in the order a picker lists them.
pub const PRIORITIES: [&str; 4] = ["low", "medium", "high", "urgent"];
/// `""` first: a task need not have a size.
pub const SIZES: [&str; 4] = ["", "S", "M", "L"];
/// [`PRIORITIES`] most urgent first, the order the board groups and offers them in.
pub const URGENT_FIRST: [&str; 4] = {
    let mut urgent_first = PRIORITIES;
    let mut i = 0;
    while i < PRIORITIES.len() {
        urgent_first[i] = PRIORITIES[PRIORITIES.len() - 1 - i];
        i += 1;
    }
    urgent_first
};

/// The board's filters: per key, the values any one of which a task must have.
pub type Filters = BTreeMap<String, BTreeSet<String>>;

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Zone {
    Before,
    Into,
    After,
}
/// Where a drop lands on a card: its top and bottom 30% insert beside it, the middle nests.
pub fn zone(y: f64, height: f64) -> Zone {
    if height <= 0. || y < height * 0.3 {
        Zone::Before
    } else if y > height * 0.7 {
        Zone::After
    } else {
        Zone::Into
    }
}

/// The index `moving` should end at when dropped beside `onto`, in a column's board order
/// (`task.move` positions are final indices among the column's other tasks).
pub fn drop_index(order: &[i64], moving: i64, onto: i64, after: bool) -> usize {
    let others: Vec<_> = order.iter().filter(|id| **id != moving).collect();
    others
        .iter()
        .position(|id| **id == onto)
        .map(|at| at + after as usize)
        .unwrap_or(others.len())
}

/// Every task of `column` in board order, filters and grouping ignored: the order `task.move`
/// positions index.
pub fn column_order(tasks: &[Value], column: &str) -> Vec<i64> {
    let mut order: Vec<_> = tasks
        .iter()
        .filter(|t| text(t, "column") == column)
        .map(|t| (t["position"].as_i64().unwrap_or(0), t["id"].as_i64().unwrap_or(0)))
        .collect();
    order.sort();
    order.into_iter().map(|(_, id)| id).collect()
}

pub fn task_matches(task: &Value, filters: &Filters, query: &str) -> bool {
    filters.iter().all(|(key, values)| {
        values.is_empty()
            || values.iter().any(|value| match key.as_str() {
                "parent" if value == "roots" => task["parent_id"].is_null(),
                "parent" | "module" => task[if key == "parent" { "parent_id" } else { "module_id" }]
                    .as_i64()
                    .is_some_and(|id| id.to_string() == *value),
                "label" | "session" => task[if key == "label" { "labels" } else { "sessions" }]
                    .as_array()
                    .is_some_and(|list| list.iter().any(|v| v.as_str() == Some(value))),
                key => text(task, key) == value,
            })
    }) && (query.is_empty() || haystack(task).contains(query))
}
/// What the search box matches against, lowercased: the id, title, body, labels, agents and
/// module name. A query is expected lowercased too.
pub fn haystack(task: &Value) -> String {
    let join = |key: &str| {
        task[key]
            .as_array()
            .map(|list| list.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
    };
    format!(
        "#{} {} {} {} {} {}",
        task["id"],
        text(task, "title"),
        text(task, "body"),
        join("labels"),
        join("sessions"),
        text(task, "module_name")
    )
    .to_lowercase()
}

/// A group's sort rank, a space, then its heading: priority and size keep their own order and
/// the fallback groups (`Unassigned`, `No module`…) sort after every named one.
pub fn group_of(task: &Value, grouping: &str) -> String {
    let named = |name: Option<&str>, none: &str| match name {
        Some(name) => format!("0 {name}"),
        None => format!("9 {none}"),
    };
    match grouping {
        "parent" => named(task["_parent_title"].as_str(), "Top level"),
        "module" => named(task["module_name"].as_str(), "No module"),
        "session" => named(task["sessions"].as_array().and_then(|v| v.last()).and_then(Value::as_str), "Unassigned"),
        "" => String::new(),
        "priority" => {
            let p = text(task, "priority");
            format!("{} {p}", URGENT_FIRST.iter().position(|x| *x == p).unwrap_or(9))
        }
        "size" => match text(task, "size") {
            "" => "9 None".into(),
            s => format!("{} {s}", SIZES.iter().position(|x| *x == s).unwrap_or(8)),
        },
        key => named(task[key].as_str(), "None"),
    }
}
/// The heading of a [`group_of`] key, its rank left off.
pub fn group_title(group: &str) -> String {
    group.split_once(' ').map(|(_, g)| g).unwrap_or(group).to_string()
}

/// The cards of `column` among `visible` (the tasks the filters keep), in the order its lane
/// shows them: by group first, then board position, then id.
pub fn sorted<'a>(visible: &[&'a Value], column: &str, grouping: &str) -> Vec<&'a Value> {
    let mut tasks: Vec<&Value> = visible
        .iter()
        .copied()
        .filter(|t| text(t, "column") == column)
        .collect();
    tasks.sort_by(|a, b| {
        group_of(a, grouping)
            .cmp(&group_of(b, grouping))
            .then_with(|| a["position"].as_i64().cmp(&b["position"].as_i64()))
            .then_with(|| a["id"].as_i64().cmp(&b["id"].as_i64()))
    });
    tasks
}

/// What a Shift+J/K nudge does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nudge {
    /// The card is first or last in its lane as shown (or not in it): nothing moves.
    Edge,
    /// The neighbour is in another group, which a nudge does not cross.
    CrossesGroup,
    /// Move the card to this final index among its column's other tasks.
    To(usize),
}
/// The focused card `id` swapping places with its neighbour `delta` away in `lane`, the lane as
/// shown (filtered tasks left out, ordered by group first, as [`sorted`] gives it). `order` is
/// the whole column's [`column_order`], which the returned index is into.
pub fn nudge(lane: &[&Value], order: &[i64], id: i64, delta: i64, grouping: &str) -> Nudge {
    let Some(at) = lane.iter().position(|t| t["id"].as_i64() == Some(id)) else { return Nudge::Edge };
    let neighbour = usize::try_from(at as i64 + delta).ok().and_then(|to| lane.get(to));
    let Some(neighbour) = neighbour else { return Nudge::Edge };
    if group_of(lane[at], grouping) != group_of(neighbour, grouping) {
        return Nudge::CrossesGroup;
    }
    let Some(onto) = neighbour["id"].as_i64() else { return Nudge::Edge };
    Nudge::To(drop_index(order, id, onto, delta > 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn filters(pairs: &[(&str, &str)]) -> Filters {
        let mut map = Filters::new();
        for (k, v) in pairs {
            map.entry(k.to_string()).or_default().insert(v.to_string());
        }
        map
    }
    /// What the engine's `task.move` does with a final index: the column order afterwards.
    fn moved(order: &[i64], moving: i64, at: usize) -> Vec<i64> {
        let mut order: Vec<i64> = order.iter().copied().filter(|id| *id != moving).collect();
        order.insert(at.min(order.len()), moving);
        order
    }
    fn ids(tasks: &[&Value]) -> Vec<i64> {
        tasks.iter().filter_map(|t| t["id"].as_i64()).collect()
    }
    /// One column, positions in id order: 1 urgent, 2 low, 3 urgent (module "Docs"), 4 low,
    /// 5 urgent, 6 low; 7 sits in another column.
    fn lane_fixture() -> Vec<Value> {
        let p = ["urgent", "low", "urgent", "low", "urgent", "low"];
        let mut tasks: Vec<Value> = (1..=6)
            .map(|id| json!({"id":id,"column":"ready","position":id - 1,"priority":p[id as usize - 1],"title":format!("Task {id}")}))
            .collect();
        tasks[2]["module_id"] = json!(4);
        tasks[2]["module_name"] = json!("Docs");
        tasks.push(json!({"id":7,"column":"backlog","position":0,"priority":"urgent"}));
        tasks
    }

    #[test]
    fn lens_combines_metadata_and_text_without_matching_unrelated_fields() {
        let task = json!({"id":9,"title":"Fix notes","body":"Keep drafts","type":"bug","priority":"high","size":"M","parent_id":2,"module_id":3,"labels":["native"],"sessions":["egret"]});
        assert!(task_matches(
            &task,
            &filters(&[("type", "bug"), ("label", "native"), ("session", "egret"), ("parent", "2")]),
            "drafts"
        ));
        assert!(!task_matches(&task, &filters(&[("parent", "roots")]), ""));
        assert!(!task_matches(&task, &filters(&[("module", "4")]), ""));
        assert!(task_matches(&task, &filters(&[("module", "3")]), ""));
        assert!(!task_matches(&task, &Filters::new(), "high"));
        // Several values of one key are alternatives; keys still all have to match.
        assert!(task_matches(&task, &filters(&[("type", "bug"), ("type", "chore")]), ""));
        assert!(!task_matches(&task, &filters(&[("type", "bug"), ("priority", "low")]), ""));
        assert!(task_matches(&task, &Filters::new(), "#9"));
        // A key with no values left filters nothing.
        assert!(task_matches(&task, &Filters::from([("type".to_string(), BTreeSet::new())]), ""));
    }
    #[test]
    fn search_reads_the_module_name_task_list_returns() {
        let task = json!({"id":1,"title":"Fix","module_id":3,"module_name":"Native Board","labels":["UI"]});
        assert!(haystack(&task).contains("native board"));
        assert!(task_matches(&task, &Filters::new(), "native board"));
        assert!(task_matches(&task, &Filters::new(), "ui"));
        assert!(!task_matches(&json!({"id":1,"title":"Fix"}), &Filters::new(), "native"));
    }
    #[test]
    fn drops_beside_a_card_compute_its_final_index() {
        let order = [1, 2, 3, 4];
        // Down the column: the moving card is not counted among the others.
        assert_eq!(drop_index(&order, 1, 3, false), 1);
        assert_eq!(drop_index(&order, 1, 3, true), 2);
        // Up the column.
        assert_eq!(drop_index(&order, 4, 2, false), 1);
        // From another column.
        assert_eq!(drop_index(&order, 9, 4, true), 4);
        assert_eq!(drop_index(&order, 9, 7, false), 4);
        assert_eq!(zone(1., 100.), Zone::Before);
        assert_eq!(zone(50., 100.), Zone::Into);
        assert_eq!(zone(90., 100.), Zone::After);
        // The 30% bands are exclusive at their edges, and a zero-height card only inserts.
        assert_eq!(zone(30., 100.), Zone::Into);
        assert_eq!(zone(70., 100.), Zone::Into);
        assert_eq!(zone(5., 0.), Zone::Before);
    }
    #[test]
    fn derived_orders_follow_the_one_vocabulary() {
        assert_eq!(URGENT_FIRST, ["urgent", "high", "medium", "low"]);
    }
    #[test]
    fn groups_sort_in_their_own_order_with_fallbacks_last() {
        let sizes: Vec<String> = ["L", "", "S", "M"]
            .iter()
            .map(|s| group_of(&json!({"size": s}), "size"))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        assert_eq!(sizes.iter().map(|g| group_title(g)).collect::<Vec<_>>(), ["S", "M", "L", "None"]);
        let priorities: Vec<String> = PRIORITIES
            .iter()
            .map(|p| group_of(&json!({"priority": p}), "priority"))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        assert_eq!(priorities.iter().map(|g| group_title(g)).collect::<Vec<_>>(), URGENT_FIRST);
        assert!(group_of(&json!({"sessions":[]}), "session") > group_of(&json!({"sessions":["zebra"]}), "session"));
        assert_eq!(group_title(&group_of(&json!({}), "module")), "No module");
        assert_eq!(group_title(&group_of(&json!({"module_name":"Docs"}), "module")), "Docs");
        assert!(group_of(&json!({}), "module") > group_of(&json!({"module_name":"Zeta"}), "module"));
        assert_eq!(group_title(&group_of(&json!({"_parent_title":"Epic"}), "parent")), "Epic");
        assert_eq!(group_title(&group_of(&json!({"type":"bug"}), "type")), "bug");
        assert_eq!(group_of(&json!({"type":"bug"}), ""), "");
    }
    #[test]
    fn lanes_sort_by_group_then_position_then_id() {
        let tasks = lane_fixture();
        let all: Vec<&Value> = tasks.iter().collect();
        // Ungrouped: board position, and the other column's task left out.
        assert_eq!(ids(&sorted(&all, "ready", "")), [1, 2, 3, 4, 5, 6]);
        // By priority: urgent before low, position order inside each group.
        assert_eq!(ids(&sorted(&all, "ready", "priority")), [1, 3, 5, 2, 4, 6]);
        // By module: the named module first, "No module" after it.
        assert_eq!(ids(&sorted(&all, "ready", "module")), [3, 1, 2, 4, 5, 6]);
        // Equal positions (a stale refresh) fall back to id.
        let tied = [json!({"id":8,"column":"ready","position":0}), json!({"id":2,"column":"ready","position":0})];
        assert_eq!(ids(&sorted(&tied.iter().collect::<Vec<_>>(), "ready", "")), [2, 8]);
        assert_eq!(column_order(&tied, "ready"), [2, 8]);
    }
    #[test]
    fn column_order_counts_every_task_the_filters_hide() {
        let tasks = lane_fixture();
        assert_eq!(column_order(&tasks, "ready"), [1, 2, 3, 4, 5, 6]);
        assert_eq!(column_order(&tasks, "backlog"), [7]);
        assert_eq!(column_order(&tasks, "done"), Vec::<i64>::new());
    }
    #[test]
    fn filtered_nudges_step_past_hidden_cards_into_place() {
        let tasks = lane_fixture();
        let order = column_order(&tasks, "ready");
        // Only the urgent cards are shown: 1, 3, 5.
        let f = filters(&[("priority", "urgent")]);
        let visible: Vec<&Value> = tasks.iter().filter(|t| task_matches(t, &f, "")).collect();
        let lane = sorted(&visible, "ready", "");
        assert_eq!(ids(&lane), [1, 3, 5]);
        // 1 down: it lands just after 3, past the hidden 2, and the lane as shown reads 3, 1, 5.
        let Nudge::To(at) = nudge(&lane, &order, 1, 1, "") else { panic!("expected a move") };
        let after = moved(&order, 1, at);
        assert_eq!(after, [2, 3, 1, 4, 5, 6]);
        // 5 up: just before 3, with the hidden 4 left behind it.
        let Nudge::To(at) = nudge(&lane, &order, 5, -1, "") else { panic!("expected a move") };
        assert_eq!(moved(&order, 5, at), [1, 2, 5, 3, 4, 6]);
        // The shown ends do not move, whatever is hidden beyond them.
        assert_eq!(nudge(&lane, &order, 5, 1, ""), Nudge::Edge);
        assert_eq!(nudge(&lane, &order, 1, -1, ""), Nudge::Edge);
        // A card the filter hides is not in the lane to nudge.
        assert_eq!(nudge(&lane, &order, 2, 1, ""), Nudge::Edge);
        // A search narrows the lane the same way: only 3 carries the module "Docs".
        let visible: Vec<&Value> = tasks.iter().filter(|t| task_matches(t, &Filters::new(), "docs")).collect();
        assert_eq!(ids(&sorted(&visible, "ready", "")), [3]);
    }
    #[test]
    fn grouped_nudges_keep_to_their_group_and_reorder_within_it() {
        let tasks = lane_fixture();
        let order = column_order(&tasks, "ready");
        let all: Vec<&Value> = tasks.iter().collect();
        let lane = sorted(&all, "ready", "priority");
        assert_eq!(ids(&lane), [1, 3, 5, 2, 4, 6]);
        // 3 down swaps with 5, its neighbour in the urgent group, past the low 4 between them in
        // board order; re-sorting shows urgent 1, 5, 3.
        let Nudge::To(at) = nudge(&lane, &order, 3, 1, "priority") else { panic!("expected a move") };
        let after = moved(&order, 3, at);
        assert_eq!(after, [1, 2, 4, 5, 3, 6]);
        let resorted: Vec<Value> = tasks
            .iter()
            .map(|t| {
                let mut t = t.clone();
                if let Some(p) = t["id"].as_i64().and_then(|id| after.iter().position(|o| *o == id)) {
                    t["position"] = json!(p);
                }
                t
            })
            .collect();
        assert_eq!(ids(&sorted(&resorted.iter().collect::<Vec<_>>(), "ready", "priority")), [1, 5, 3, 2, 4, 6]);
        // 5 is the last urgent card: down would cross into the low group.
        assert_eq!(nudge(&lane, &order, 5, 1, "priority"), Nudge::CrossesGroup);
        assert_eq!(nudge(&lane, &order, 2, -1, "priority"), Nudge::CrossesGroup);
        // 4 up swaps with 2 within the low group.
        let Nudge::To(at) = nudge(&lane, &order, 4, -1, "priority") else { panic!("expected a move") };
        assert_eq!(moved(&order, 4, at), [1, 4, 2, 3, 5, 6]);
        // Grouped and filtered at once: low cards only, grouped by priority, 6 up past hidden 5.
        let f = filters(&[("priority", "low")]);
        let visible: Vec<&Value> = tasks.iter().filter(|t| task_matches(t, &f, "")).collect();
        let lane = sorted(&visible, "ready", "priority");
        assert_eq!(ids(&lane), [2, 4, 6]);
        let Nudge::To(at) = nudge(&lane, &order, 6, -1, "priority") else { panic!("expected a move") };
        assert_eq!(moved(&order, 6, at), [1, 2, 3, 6, 4, 5]);
    }
    #[test]
    fn grouped_drops_land_beside_the_card_they_were_dropped_on() {
        let tasks = lane_fixture();
        let order = column_order(&tasks, "ready");
        // Grouped by priority the lane shows 1, 3, 5 | 2, 4, 6. Dropping 6 on the lower part of 3
        // places it right after 3 in board order, ahead of the 4 and 5 it skipped over.
        let at = drop_index(&order, 6, 3, zone(80., 100.) == Zone::After);
        assert_eq!(moved(&order, 6, at), [1, 2, 3, 6, 4, 5]);
        // On the upper part of 1, the first card: to the top.
        let at = drop_index(&order, 4, 1, zone(10., 100.) == Zone::After);
        assert_eq!(moved(&order, 4, at), [4, 1, 2, 3, 5, 6]);
        // From another column (7, in backlog) onto 5's lower part.
        let at = drop_index(&order, 7, 5, true);
        assert_eq!(moved(&order, 7, at), [1, 2, 3, 4, 5, 7, 6]);
    }
}
