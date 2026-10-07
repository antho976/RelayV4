//! The add / edit entry sheet, and the add account sheet.
use super::formatter;
use super::pages::{human_date, today};
use crate::app::{button, label, rows, text, Ui};
use crate::panel::Panel;
use gtk::prelude::*;
use gtk4 as gtk;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;

const KINDS: [(&str, &str); 3] = [("EXPENSE", "Expense"), ("INCOME", "Income"), ("TRANSFER", "Transfer")];

fn field(body: &gtk::Box, caption: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let block = gtk::Box::new(gtk::Orientation::Vertical, 6);
    block.append(&label(&caption.to_uppercase(), "money-label"));
    block.append(widget);
    body.append(&block);
    block
}

/// A drop-down over `(id, name)` pairs, selecting `selected` when present.
fn picker(items: &[(i64, String)], selected: Option<i64>) -> gtk::DropDown {
    let names: Vec<&str> = items.iter().map(|(_, n)| n.as_str()).collect();
    let dropdown = gtk::DropDown::from_strings(&names);
    dropdown.add_css_class("money-picker");
    if let Some(index) = selected.and_then(|id| items.iter().position(|(i, _)| *i == id)) {
        dropdown.set_selected(index as u32);
    }
    dropdown
}

fn chosen(dropdown: &gtk::DropDown, items: &[(i64, String)]) -> Option<i64> {
    items.get(dropdown.selected() as usize).map(|(id, _)| *id)
}

/// Opens the entry sheet: a new entry, or `tx` to edit.
pub fn open(ui: &Rc<Ui>, tx: Option<Value>) {
    let title = if tx.is_some() { "Edit entry" } else { "Add entry" };
    let Some(panel) = Panel::toggle(ui, title, 460) else { return };
    panel.add_css_class("money-sheet");
    let loading = label("Reading accounts and categories…", "money-muted");
    panel.body.append(&loading);
    panel.present();
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let lists = ui.call("money.lists", json!({})).await;
        loading.unparent();
        match lists {
            Ok(lists) => {
                super::remember_currency(&lists);
                form(&ui, &panel, &lists, tx);
            }
            Err(e) => {
                let l = label(&super::pages::unavailable(&e), "money-muted");
                l.set_wrap(true);
                panel.body.append(&l);
            }
        }
    });
}

fn form(ui: &Rc<Ui>, panel: &Rc<Panel>, lists: &Value, tx: Option<Value>) {
    let fmt = formatter();
    let body = &panel.body;
    let accounts: Vec<(i64, String)> = rows(lists, "accounts")
        .iter()
        .filter(|a| a["archived"] != true || tx.as_ref().is_some_and(|t| t["account_id"] == a["id"] || t["to_account_id"] == a["id"]))
        .map(|a| (a["id"].as_i64().unwrap_or(0), text(a, "name").to_string()))
        .collect();
    if accounts.is_empty() {
        let none = label(
            "There is no account to log against yet. Add one first; the first account also brings Tally's usual categories.",
            "money-muted",
        );
        none.set_wrap(true);
        body.append(&none);
        let add = button("Add an account", "primary");
        add.add_css_class("money-hero-action");
        let weak = Rc::downgrade(ui);
        let sheet = Rc::downgrade(panel);
        add.connect_clicked(move |_| {
            if let (Some(ui), Some(sheet)) = (weak.upgrade(), sheet.upgrade()) {
                sheet.close();
                account(&ui);
            }
        });
        body.append(&add);
        return;
    }
    let categories = rows(lists, "categories");
    let kind = Rc::new(RefCell::new(tx.as_ref().map_or("EXPENSE", |t| text(t, "type")).to_string()));

    // Expense, Income, Transfer.
    let segments = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    segments.add_css_class("money-segments");
    segments.set_homogeneous(true);
    let mut group: Option<gtk::ToggleButton> = None;
    let mut toggles = Vec::new();
    for (value, caption) in KINDS {
        let key = gtk::ToggleButton::with_label(caption);
        key.set_widget_name(&format!("money-kind-{}", value.to_lowercase()));
        key.set_group(group.as_ref());
        if group.is_none() {
            group = Some(key.clone());
        }
        key.set_active(*kind.borrow() == value);
        segments.append(&key);
        toggles.push((value, key));
    }
    body.append(&segments);

    // The amount, in the serif figure voice, with the currency beside it.
    let amount_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    amount_row.add_css_class("money-amount-row");
    let amount = gtk::Entry::new();
    amount.add_css_class("money-amount-field");
    amount.set_widget_name("money-entry-amount");
    amount.set_hexpand(true);
    amount.set_placeholder_text(Some(&fmt.format_input(0)));
    amount.set_input_purpose(gtk::InputPurpose::Number);
    amount.update_property(&[gtk::accessible::Property::Label("Amount")]);
    if let Some(t) = &tx {
        amount.set_text(&fmt.format_input(t["amount"].as_i64().unwrap_or(0)));
    }
    amount_row.append(&amount);
    let code = label(&fmt.currency, "money-suffix");
    code.set_valign(gtk::Align::Center);
    amount_row.append(&code);
    field(body, "Amount", &amount_row);
    let problem = label("", "money-problem");
    problem.set_wrap(true);
    problem.set_visible(false);
    body.append(&problem);

    // Category, of the kind that matches; transfers take none.
    let category_items: Rc<RefCell<Vec<(i64, String)>>> = Rc::default();
    let category_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let category_block = field(body, "Category", &category_slot);
    let category: Rc<RefCell<Option<gtk::DropDown>>> = Rc::default();
    let fill_categories = {
        let (items, slot, category, block, kind) = (category_items.clone(), category_slot.clone(), category.clone(), category_block.clone(), kind.clone());
        let selected = tx.as_ref().and_then(|t| t["category_id"].as_i64());
        move || {
            let kind = kind.borrow().clone();
            block.set_visible(kind != "TRANSFER");
            let list: Vec<(i64, String)> = categories
                .iter()
                .filter(|c| text(c, "kind") == kind && (c["archived"] != true || c["id"].as_i64() == selected))
                .map(|c| (c["id"].as_i64().unwrap_or(0), text(c, "name").to_string()))
                .collect();
            if let Some(old) = category.borrow_mut().take() {
                slot.remove(&old);
            }
            let dropdown = picker(&list, selected);
            dropdown.set_widget_name("money-entry-category");
            slot.append(&dropdown);
            *category.borrow_mut() = Some(dropdown);
            *items.borrow_mut() = list;
        }
    };
    fill_categories();

    let from = picker(&accounts, tx.as_ref().and_then(|t| t["account_id"].as_i64()));
    from.set_widget_name("money-entry-account");
    let from_block = field(body, "Account", &from);
    let to = picker(&accounts, tx.as_ref().and_then(|t| t["to_account_id"].as_i64()).or_else(|| accounts.get(1).map(|a| a.0)));
    to.set_widget_name("money-entry-to-account");
    let to_block = field(body, "To account", &to);
    to_block.set_visible(*kind.borrow() == "TRANSFER");
    let from_caption = from_block.first_child().and_downcast::<gtk::Label>();
    let set_from_caption = move |transfer: bool| {
        if let Some(caption) = &from_caption {
            caption.set_text(if transfer { "FROM ACCOUNT" } else { "ACCOUNT" });
        }
    };
    set_from_caption(*kind.borrow() == "TRANSFER");

    // The date: today unless changed, picked on a calendar.
    let day = Rc::new(RefCell::new(tx.as_ref().map_or_else(today, |t| text(t, "date").to_string())));
    let date_key = gtk::MenuButton::new();
    date_key.add_css_class("money-picker");
    date_key.set_widget_name("money-entry-date");
    date_key.set_label(&human_date(&day.borrow()));
    let calendar = gtk::Calendar::new();
    if let Some(selected) = parse_day(&day.borrow()) {
        calendar.set_date(&selected);
    }
    let popover = gtk::Popover::new();
    popover.set_child(Some(&calendar));
    date_key.set_popover(Some(&popover));
    {
        let (day, date_key, popover) = (day.clone(), date_key.clone(), popover.clone());
        calendar.connect_day_selected(move |calendar| {
            let picked = calendar.date().format("%Y-%m-%d").map(|s| s.to_string()).unwrap_or_default();
            date_key.set_label(&human_date(&picked));
            *day.borrow_mut() = picked;
            popover.popdown();
        });
    }
    field(body, "Date", &date_key);

    let note = gtk::Entry::new();
    note.add_css_class("money-field");
    note.set_widget_name("money-entry-note");
    note.set_placeholder_text(Some("What it was, if the category does not say"));
    if let Some(t) = &tx {
        note.set_text(text(t, "note"));
    }
    field(body, "Note", &note);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    actions.add_css_class("money-sheet-actions");
    let save = button("", "primary");
    save.add_css_class("money-hero-action");
    save.set_widget_name("money-entry-save");
    save.set_hexpand(true);
    let caption = move |kind: &str| match kind {
        "INCOME" => "Save income",
        "TRANSFER" => "Save transfer",
        _ => "Save expense",
    };
    save.set_label(caption(&kind.borrow()));
    if let Some(t) = &tx {
        let delete = button("Delete", "money-destructive");
        delete.set_widget_name("money-entry-delete");
        let weak = Rc::downgrade(ui);
        let sheet = Rc::downgrade(panel);
        let gone = t.clone();
        delete.connect_clicked(move |_| {
            if let (Some(ui), Some(sheet)) = (weak.upgrade(), sheet.upgrade()) {
                sheet.close();
                super::pages::delete(&ui, gone.clone());
            }
        });
        actions.append(&delete);
    }
    actions.append(&save);
    body.append(&actions);

    for (value, key) in &toggles {
        let (kind, fill, to_block, save, value) = (kind.clone(), fill_categories.clone(), to_block.clone(), save.clone(), *value);
        let set_from_caption = set_from_caption.clone();
        key.connect_toggled(move |key| {
            if !key.is_active() || *kind.borrow() == value {
                return;
            }
            *kind.borrow_mut() = value.to_string();
            fill();
            to_block.set_visible(value == "TRANSFER");
            set_from_caption(value == "TRANSFER");
            save.set_label(caption(value));
        });
    }

    let submit = {
        let ui = ui.clone();
        let sheet = Rc::downgrade(panel);
        let (amount, problem, kind, category, category_items, from, to, day, note) =
            (amount.clone(), problem.clone(), kind.clone(), category.clone(), category_items.clone(), from.clone(), to.clone(), day.clone(), note.clone());
        let accounts = accounts.clone();
        let id = tx.as_ref().map(|t| t["id"].clone());
        let save = save.clone();
        move || {
            let refuse = |text: &str| {
                problem.set_text(text);
                problem.set_visible(true);
                amount.add_css_class("error");
            };
            let Some(value) = formatter().parse(&amount.text()).filter(|v| *v > 0) else {
                refuse("Type an amount above zero, like 12.50.");
                amount.grab_focus();
                return;
            };
            amount.remove_css_class("error");
            let kind = kind.borrow().clone();
            let Some(account) = chosen(&from, &accounts) else {
                refuse("Pick the account this came from.");
                return;
            };
            let mut payload = json!({"type":kind,"amount":value,"date":*day.borrow(),"account_id":account});
            if kind == "TRANSFER" {
                let target = chosen(&to, &accounts);
                if target.is_none_or(|t| t == account) {
                    refuse("A transfer goes to a different account.");
                    return;
                }
                payload["to_account_id"] = json!(target);
            } else {
                let picked = category.borrow().as_ref().and_then(|d| chosen(d, &category_items.borrow()));
                match picked {
                    Some(c) => payload["category_id"] = json!(c),
                    None => {
                        refuse("Pick a category. Tally keeps every expense and income in one.");
                        return;
                    }
                }
            }
            payload["note"] = json!(note.text().trim());
            let op = match &id {
                Some(id) => {
                    payload["id"] = id.clone();
                    "money.tx.update"
                }
                None => "money.tx.add",
            };
            problem.set_visible(false);
            save.set_sensitive(false);
            let (ui, sheet, problem, save) = (ui.clone(), sheet.clone(), problem.clone(), save.clone());
            glib::spawn_future_local(async move {
                match ui.call(op, payload).await {
                    Ok(_) => {
                        if let Some(sheet) = sheet.upgrade() {
                            sheet.close();
                        }
                        ui.refresh_page();
                    }
                    Err(e) => {
                        problem.set_text(&e.to_string());
                        problem.set_visible(true);
                        save.set_sensitive(true);
                    }
                }
            });
        }
    };
    let submit = Rc::new(submit);
    let run = submit.clone();
    save.connect_clicked(move |_| run());
    let run = submit.clone();
    amount.connect_activate(move |_| run());
    note.connect_activate(move |_| submit());
    amount.grab_focus();
}

fn parse_day(value: &str) -> Option<glib::DateTime> {
    let mut parts = value.split('-').map(|p| p.parse::<i32>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    glib::DateTime::from_local(y, m, d, 0, 0, 0.0).ok()
}

const ACCOUNT_TYPES: [(&str, &str); 5] = [
    ("CHEQUING", "Chequing"),
    ("SAVINGS", "Savings"),
    ("CASH", "Cash"),
    ("CREDIT", "Credit card"),
    ("INVESTMENT", "Investment"),
];

/// The add account sheet: a name, a type and what it holds today.
pub fn account(ui: &Rc<Ui>) {
    let Some(panel) = Panel::toggle(ui, "Add an account", 460) else { return };
    panel.add_css_class("money-sheet");
    let body = &panel.body;
    let fmt = formatter();
    let name = gtk::Entry::new();
    name.add_css_class("money-field");
    name.set_widget_name("money-account-name");
    name.set_placeholder_text(Some("Everyday chequing"));
    field(body, "Name", &name);
    let kinds: Vec<&str> = ACCOUNT_TYPES.iter().map(|(_, caption)| *caption).collect();
    let kind = gtk::DropDown::from_strings(&kinds);
    kind.add_css_class("money-picker");
    kind.set_widget_name("money-account-type");
    field(body, "Type", &kind);
    let balance_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let balance = gtk::Entry::new();
    balance.add_css_class("money-field");
    balance.set_widget_name("money-account-balance");
    balance.set_hexpand(true);
    balance.set_placeholder_text(Some(&fmt.format_input(0)));
    balance_row.append(&balance);
    balance_row.append(&label(&fmt.currency, "money-suffix"));
    field(body, "Holds today", &balance_row);
    let owed = gtk::CheckButton::with_label("This is money owed, such as a card balance");
    owed.set_widget_name("money-account-owed");
    body.append(&owed);
    let weak_owed = owed.downgrade();
    kind.connect_selected_notify(move |kind| {
        if let Some(owed) = weak_owed.upgrade() {
            owed.set_active(ACCOUNT_TYPES[kind.selected() as usize].0 == "CREDIT");
        }
    });
    let problem = label("", "money-problem");
    problem.set_wrap(true);
    problem.set_visible(false);
    body.append(&problem);
    let save = button("Add account", "primary");
    save.add_css_class("money-hero-action");
    save.set_widget_name("money-account-save");
    body.append(&save);
    let submit = {
        let ui = ui.clone();
        let sheet = Rc::downgrade(&panel);
        let (name, kind, balance, owed, problem, save) = (name.clone(), kind.clone(), balance.clone(), owed.clone(), problem.clone(), save.clone());
        move || {
            let title = name.text().trim().to_string();
            if title.is_empty() {
                problem.set_text("Give the account a name.");
                problem.set_visible(true);
                name.grab_focus();
                return;
            }
            let typed = balance.text();
            let opening = if typed.trim().is_empty() { Some(0) } else { formatter().parse(&typed) };
            let Some(opening) = opening else {
                problem.set_text("Type what it holds as an amount, like 1250.00. Tick the box below for money owed.");
                problem.set_visible(true);
                return;
            };
            let opening = if owed.is_active() { -opening } else { opening };
            let payload = json!({"name":title,"type":ACCOUNT_TYPES[kind.selected() as usize].0,"opening_balance":opening});
            save.set_sensitive(false);
            problem.set_visible(false);
            let (ui, sheet, problem, save) = (ui.clone(), sheet.clone(), problem.clone(), save.clone());
            glib::spawn_future_local(async move {
                match ui.call("money.account.add", payload).await {
                    Ok(v) => {
                        if let Some(sheet) = sheet.upgrade() {
                            sheet.close();
                        }
                        super::toast(&ui, &format!("Added {}.", text(&v, "name")), None);
                        ui.refresh_page();
                    }
                    Err(e) => {
                        problem.set_text(&e.to_string());
                        problem.set_visible(true);
                        save.set_sensitive(true);
                    }
                }
            });
        }
    };
    let submit = Rc::new(submit);
    let run = submit.clone();
    save.connect_clicked(move |_| run());
    let run = submit.clone();
    name.connect_activate(move |_| run());
    balance.connect_activate(move |_| submit());
    panel.present();
    name.grab_focus();
}
