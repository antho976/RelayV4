//! Arbiter across the bus, on a fake exchange: strategies on paper and live, the runner's
//! decisions, the risk gate, proposals and approvals, the kill switch, and the key check.

mod common;

use common::{call, code, engine, ok};
use relay_arbiter::exchange::{Account, ExchangeError, Market, Result};
use relay_arbiter::model::*;
use relay_arbiter::Decimal;
use relay_core::arbiter::{pass, set_fake_exchange};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const KEY: &str = "-----BEGIN EC PRIVATE KEY-----
MHcCAQEEICr3hFJQLruuvjzXzQvrZyWUlBfa0SdE9HtDCMhP7NYqoAoGCCqGSM49
AwEHoUQDQgAEIgIeCM0SMcTdKeIUVa5s6GrENsIc3sFPBcefipt5Ciw6PtUG6nMV
TsGghZLrhr8kSV5nZX8i1b+Q+3+QzuY0wQ==
-----END EC PRIVATE KEY-----";

/// An exchange with one product whose price the test sets.
#[derive(Default)]
struct Fake {
    price: Mutex<f64>,
    can_transfer: Mutex<bool>,
    placed: Mutex<Vec<OrderRequest>>,
}

impl Fake {
    fn new(price: f64) -> Arc<Fake> {
        let f = Fake::default();
        *f.price.lock().unwrap() = price;
        Arc::new(f)
    }
    fn set(&self, price: f64) {
        *self.price.lock().unwrap() = price;
    }
    fn now() -> i64 {
        jiff::Timestamp::now().as_second()
    }
}

fn eth() -> Product {
    Product {
        id: "ETH-CAD".into(),
        base: "ETH".into(),
        quote: "CAD".into(),
        base_increment: Decimal::new(1, 8),
        quote_increment: Decimal::new(1, 2),
        price_increment: Decimal::new(1, 2),
        base_min_size: Decimal::new(1, 8),
        quote_min_size: Decimal::ONE,
        price: Some(100.0),
        change_24h: Some(1.5),
        tradable: true,
        limit_only: false,
    }
}

impl Market for Fake {
    fn products(&self) -> Result<Vec<Product>> {
        Ok(vec![eth()])
    }
    fn product(&self, id: &str) -> Result<Product> {
        if id == "ETH-CAD" { Ok(eth()) } else { Err(ExchangeError::Refused(format!("no {id}"))) }
    }
    fn candles(&self, _: &str, g: Granularity, start: i64, end: i64) -> Result<Vec<Candle>> {
        let p = *self.price.lock().unwrap();
        let s = g.seconds();
        let first = start.div_euclid(s) * s;
        Ok((0..).map(|i| first + i * s).take_while(|t| *t < end && *t <= Fake::now()).filter(|t| *t >= start)
            .map(|t| Candle { start: t, open: p, high: p, low: p, close: p, volume: 1.0 }).collect())
    }
    fn quote(&self, _: &str) -> Result<Quote> {
        let p = *self.price.lock().unwrap();
        Ok(Quote { bid: p - 0.05, ask: p + 0.05, at: Fake::now() })
    }
}

impl Account for Fake {
    fn permissions(&self) -> Result<Permissions> {
        Ok(Permissions { can_view: true, can_trade: true, can_transfer: *self.can_transfer.lock().unwrap(), portfolio_uuid: Some("p1".into()), portfolio_type: Some("DEFAULT".into()) })
    }
    fn balances(&self) -> Result<Vec<Balance>> {
        Ok(vec![Balance { currency: "CAD".into(), available: Decimal::new(500, 0), hold: Decimal::ZERO, value: None }])
    }
    fn fees(&self) -> Result<Fees> {
        Ok(Fees { maker: Decimal::new(4, 3), taker: Decimal::new(6, 3), tier: Some("Intro".into()) })
    }
    fn preview(&self, _: &OrderRequest) -> Result<Preview> {
        Ok(Preview { preview_id: Some("pv".into()), commission: None, quote_size: None, base_size: None, errors: vec![], warnings: vec![] })
    }
    fn place(&self, o: &OrderRequest, _: Option<&str>) -> Result<ExchangeOrder> {
        self.placed.lock().unwrap().push(o.clone());
        Ok(ExchangeOrder { order_id: format!("x-{}", o.client_order_id), client_order_id: o.client_order_id.clone(), status: OrderStatus::Pending, filled_size: Decimal::ZERO, average_filled_price: Decimal::ZERO, total_fees: Decimal::ZERO, reason: None })
    }
    fn order(&self, id: &str) -> Result<ExchangeOrder> {
        let p = Decimal::try_from(*self.price.lock().unwrap()).unwrap();
        Ok(ExchangeOrder { order_id: id.into(), client_order_id: id.trim_start_matches("x-").into(), status: OrderStatus::Filled, filled_size: Decimal::new(5, 1), average_filled_price: p, total_fees: Decimal::new(3, 1), reason: None })
    }
    fn fills(&self, id: &str) -> Result<Vec<ExchangeFill>> {
        let p = Decimal::try_from(*self.price.lock().unwrap()).unwrap();
        Ok(vec![ExchangeFill { trade_id: format!("t-{id}"), order_id: id.into(), price: p, size: Decimal::new(5, 1), fee: Decimal::new(3, 1), at: Fake::now() }])
    }
    fn cancel(&self, _: &[String]) -> Result<()> {
        Ok(())
    }
}

/// A rule that buys on every closed bar while nothing is held, and sells on a 5% fall or 5% rise.
fn draft(name: &str, buy: &str) -> Value {
    json!({
        "name": name, "product": "ETH-CAD", "granularity": "ONE_MINUTE",
        "rule": {
            "entry": {"kind": "signal", "when": {"kind": "compare", "left": {"kind": "price"}, "op": "below", "right": {"kind": "number", "value": 1_000_000}}},
            "buy": buy, "pricing": "market",
            "exit": {"take_profit_pct": "5", "stop_loss_pct": "5"}
        },
        "limits": {"max_order": "50", "max_position": "200", "daily_loss": "40"}
    })
}

fn setup() -> (Arc<relay_core::Engine>, Arc<Fake>) {
    let engine = engine();
    let fake = Fake::new(100.0);
    set_fake_exchange(&engine, fake.clone());
    (engine, fake)
}

#[test]
fn a_paper_strategy_buys_on_its_bar_and_its_stop_sells() {
    let (engine, fake) = setup();
    let s = ok(&engine, "arbiter.strategy.save", json!({"draft": draft("Dip", "50")}));
    let id = s["row"]["id"].as_i64().unwrap();
    assert_eq!(s["row"]["venue"], "paper");
    assert_eq!(s["row"]["state"], "stopped");
    ok(&engine, "arbiter.strategy.set", json!({"id": id, "running": true}));
    pass(&engine);
    let d = ok(&engine, "arbiter.strategy.get", json!({"id": id}));
    assert_eq!(d["fills"].as_array().unwrap().len(), 1, "{d:#}");
    assert!(d["row"]["held_base"].as_str().unwrap().parse::<f64>().unwrap() > 0.0);
    // A second pass on the same bar decides nothing new.
    pass(&engine);
    assert_eq!(ok(&engine, "arbiter.strategy.get", json!({"id": id}))["fills"].as_array().unwrap().len(), 1);
    // The price falls 10%: the stop sells between bars.
    fake.set(90.0);
    pass(&engine);
    let d = ok(&engine, "arbiter.strategy.get", json!({"id": id}));
    assert_eq!(d["row"]["held_base"], "0");
    assert_eq!(d["paper"]["trades"], 1);
    assert!(d["paper"]["pnl"].as_str().unwrap().starts_with('-'));
    let kinds: Vec<String> = d["decisions"].as_array().unwrap().iter().map(|x| x["kind"].as_str().unwrap().to_string()).collect();
    assert!(kinds.contains(&"buy".to_string()) && kinds.contains(&"sell".to_string()), "{kinds:?}");
    let summary = ok(&engine, "arbiter.summary", json!({}));
    assert_eq!(summary["connection"]["state"], "none");
    assert!(summary["paper"]["value"].as_str().unwrap().parse::<f64>().unwrap() < 1000.0);
}

#[test]
fn an_agents_order_is_a_proposal_unless_the_ai_may_trade() {
    let (engine, _) = setup();
    let id = ok(&engine, "arbiter.strategy.save", json!({"draft": draft("Agent", "20")}))["row"]["id"].as_i64().unwrap();
    let out = ok(&engine, "arbiter.order.place", json!({"strategy_id": id, "side": "buy", "quote": "20", "why": "RSI is low"}));
    assert_eq!(out["outcome"], "proposed", "{out:#}");
    let pid = out["proposal"]["id"].as_i64().unwrap();
    let approved = ok(&engine, "arbiter.proposal.resolve", json!({"id": pid, "approve": true}));
    assert_eq!(approved["status"], "approved", "{approved:#}");
    assert_eq!(ok(&engine, "arbiter.order.list", json!({"strategy_id": id}))["orders"][0]["status"], "filled");
    // Answered once only.
    assert_eq!(code(call(&engine, "arbiter.proposal.resolve", json!({"id": pid, "approve": true}))), "arbiter.answered");

    // Let the AI trade: its orders now place, and the gate still refuses what is over a limit.
    ok(&engine, "arbiter.strategy.set", json!({"id": id, "mode": "agent"}));
    let out = ok(&engine, "arbiter.order.place", json!({"strategy_id": id, "side": "buy", "quote": "60", "why": "bigger"}));
    assert_eq!(out["outcome"], "refused");
    assert_eq!(out["refusal"]["code"], "arbiter.over_max_order");
    let out = ok(&engine, "arbiter.order.place", json!({"strategy_id": id, "side": "sell", "why": "take it"}));
    assert_eq!(out["outcome"], "placed", "{out:#}");
}

#[test]
fn the_ai_may_not_trade_without_limits_and_cannot_invent_products() {
    let (engine, _) = setup();
    let mut d = draft("Bare", "20");
    d["limits"] = json!({});
    let id = ok(&engine, "arbiter.strategy.save", json!({"draft": d}))["row"]["id"].as_i64().unwrap();
    assert_eq!(code(call(&engine, "arbiter.strategy.set", json!({"id": id, "mode": "agent"}))), "arbiter.invalid");
    let mut moon = draft("Moon", "20");
    moon["product"] = json!("MOON-CAD");
    assert_eq!(code(call(&engine, "arbiter.propose", json!({"draft": moon, "why": "to the moon"}))), "arbiter.unknown_product");
    assert_eq!(code(call(&engine, "arbiter.strategy.save", json!({"draft": moon}))), "arbiter.unknown_product");
}

#[test]
fn a_proposed_change_applies_exactly_as_shown() {
    let (engine, _) = setup();
    let id = ok(&engine, "arbiter.strategy.save", json!({"draft": draft("Dip", "50")}))["row"]["id"].as_i64().unwrap();
    let mut next = draft("Dip", "50");
    next["rule"]["exit"]["take_profit_pct"] = json!("3");
    let p = ok(&engine, "arbiter.propose", json!({"strategy_id": id, "draft": next, "why": "A smaller target won more often in the backtest"}));
    assert_eq!(p["kind"], "change");
    assert!(p["changes"][0].as_str().unwrap().contains("up 3%"), "{p:#}");
    assert_eq!(code(call(&engine, "arbiter.proposal.resolve", json!({"id": p["id"], "approve": true, "draft_hash": "not-it"}))), "arbiter.changed_since");
    ok(&engine, "arbiter.proposal.resolve", json!({"id": p["id"], "approve": true, "draft_hash": p["draft_hash"]}));
    let d = ok(&engine, "arbiter.strategy.get", json!({"id": id}));
    assert_eq!(d["row"]["version"], 2);
    assert_eq!(d["rule"]["exit"]["take_profit_pct"], "3");
}

#[test]
fn the_kill_switch_stops_everything_until_restarted() {
    let (engine, _) = setup();
    let id = ok(&engine, "arbiter.strategy.save", json!({"draft": draft("Dip", "50")}))["row"]["id"].as_i64().unwrap();
    ok(&engine, "arbiter.strategy.set", json!({"id": id, "running": true}));
    ok(&engine, "arbiter.halt", json!({"reason": "testing"}));
    pass(&engine);
    assert!(ok(&engine, "arbiter.strategy.get", json!({"id": id}))["fills"].as_array().unwrap().is_empty());
    assert_eq!(ok(&engine, "arbiter.summary", json!({}))["halted"], true);
    ok(&engine, "arbiter.restart", json!({}));
    pass(&engine);
    assert_eq!(ok(&engine, "arbiter.strategy.get", json!({"id": id}))["fills"].as_array().unwrap().len(), 1);
}

#[test]
fn a_key_that_can_transfer_is_refused_and_live_needs_a_key() {
    let (engine, fake) = setup();
    let id = ok(&engine, "arbiter.strategy.save", json!({"draft": draft("Live", "50")}))["row"]["id"].as_i64().unwrap();
    assert_eq!(code(call(&engine, "arbiter.strategy.set", json!({"id": id, "venue": "live"}))), "arbiter.not_connected");
    *fake.can_transfer.lock().unwrap() = true;
    assert_eq!(code(call(&engine, "arbiter.key.set", json!({"key_name": "organizations/o/apiKeys/k1", "private_key": KEY}))), "arbiter.key_can_transfer");
    *fake.can_transfer.lock().unwrap() = false;
    let c = ok(&engine, "arbiter.key.set", json!({"key_name": "organizations/o/apiKeys/k1234", "private_key": KEY}));
    assert_eq!(c["state"], "ok");
    assert_eq!(c["key"], "organizations/…/apiKeys/k123…");
    assert_eq!(c["fees"]["taker"], "0.006");

    // Live: preview, place, then the fills come back from the exchange.
    ok(&engine, "arbiter.strategy.set", json!({"id": id, "venue": "live", "running": true}));
    pass(&engine);
    assert_eq!(fake.placed.lock().unwrap().len(), 1);
    let d = ok(&engine, "arbiter.strategy.get", json!({"id": id}));
    assert_eq!(d["orders"][0]["status"], "filled", "{d:#}");
    assert_eq!(d["live"]["trades"], 0);
    assert_eq!(d["row"]["held_base"], "0.5");
    let summary = ok(&engine, "arbiter.summary", json!({}));
    assert_eq!(summary["live"]["cash"], "500");
}

#[test]
fn a_backtest_reports_both_parts_and_counts_versions() {
    let (engine, _) = setup();
    let id = ok(&engine, "arbiter.strategy.save", json!({"draft": draft("Dip", "50")}))["row"]["id"].as_i64().unwrap();
    let r = ok(&engine, "arbiter.backtest", json!({"strategy_id": id, "days": 1}));
    assert!(r["judged"]["start"].as_i64().unwrap() > r["tuned"]["start"].as_i64().unwrap());
    let mut other = draft("Dip", "50");
    other["rule"]["exit"]["take_profit_pct"] = json!("2");
    let r = ok(&engine, "arbiter.backtest", json!({"strategy_id": id, "draft": other, "days": 1}));
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().starts_with("2 versions")), "{r:#}");
    assert_eq!(ok(&engine, "arbiter.strategy.get", json!({"id": id}))["variants_tried"], 2);
}
