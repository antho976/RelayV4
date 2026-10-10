//! The risk gate: every order, from a rule, an agent or the person, passes [`check`] before it
//! reaches an exchange, paper or live. It refuses rather than resizes — a limit that shrinks an
//! order is not a limit — and it is the only place that turns an intent into an order, so sizes
//! and prices always meet the product's increments.

use crate::model::{GlobalLimits, Limits, OrderRequest, Product, Quote, Side, Source};
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::{Decimal, RoundingStrategy};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A price older than this is not one to trade on, seconds.
pub const STALE_QUOTE: i64 = 120;

/// What someone wants to do.
#[derive(Debug, Clone, PartialEq)]
pub struct Intent {
    pub product: String,
    pub side: Side,
    /// Buys: how much quote currency to spend.
    pub quote: Option<Decimal>,
    /// Sells: how much of the base to sell; absent sells the whole holding.
    pub base: Option<Decimal>,
    /// A post-only limit at the best price on this side, instead of a market order.
    pub limit: bool,
    pub source: Source,
    /// What caused it, unique per decision ("strategy 4, bar 1760000000, buy"): the
    /// `client_order_id` is derived from it, so the same decision can never place two orders.
    pub cause: String,
}

/// Everything the gate weighs, read fresh for each intent.
#[derive(Debug, Clone)]
pub struct Situation<'a> {
    /// The product as the exchange describes it now; `None` when it does not exist.
    pub product: Option<&'a Product>,
    pub quote: Option<Quote>,
    pub now: i64,
    /// The strategy's own limits.
    pub limits: &'a Limits,
    pub global: &'a GlobalLimits,
    /// What this strategy holds: base amount and its cost.
    pub held_base: Decimal,
    pub held_cost: Decimal,
    /// What every strategy on this venue holds, at cost.
    pub exposure: Decimal,
    /// Losses today as positive numbers (realized, plus what open positions are down).
    pub loss_today: Decimal,
    pub loss_today_all: Decimal,
    pub orders_last_hour: u32,
    pub orders_last_hour_all: u32,
    /// When this strategy last sold at a loss, Unix seconds.
    pub last_loss_at: Option<i64>,
    pub halted: bool,
    pub all_halted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "ArbiterRefusal")]
pub struct Refusal {
    /// `arbiter.unknown_product`, `arbiter.over_max_order`…
    pub code: String,
    pub message: String,
}

fn refuse(code: &str, message: impl Into<String>) -> Refusal {
    Refusal { code: format!("arbiter.{code}"), message: message.into() }
}

/// A `client_order_id` from a cause: the same cause, the same id, so the exchange returns the
/// order it already has instead of placing a second (UUID-shaped, as the exchange expects).
pub fn client_order_id(cause: &str) -> String {
    let d = Sha256::digest(cause.as_bytes());
    let h: String = d.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn floor_to(x: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO {
        return x;
    }
    ((x / step).round_dp_with_strategy(0, RoundingStrategy::ToZero) * step).normalize()
}

fn money(d: Decimal) -> String {
    d.round_dp(2).normalize().to_string()
}

/// Turns an intent into an order the exchange will take, or says plainly why not.
pub fn check(intent: &Intent, s: &Situation) -> Result<OrderRequest, Refusal> {
    let person = intent.source == Source::Person;
    // Halts stop everything but the person's own sells (flattening after the kill switch).
    if (s.all_halted || s.halted) && !(person && intent.side == Side::Sell) {
        return Err(refuse("halted", if s.all_halted { "Arbiter is halted. Restart it before trading." } else { "This strategy is halted. Restart it before trading." }));
    }
    let Some(p) = s.product else {
        return Err(refuse("unknown_product", format!("{} is not a product on the exchange.", intent.product)));
    };
    if p.id != intent.product {
        return Err(refuse("unknown_product", format!("{} is not this strategy's product ({}).", intent.product, p.id)));
    }
    if !p.tradable {
        return Err(refuse("not_tradable", format!("The exchange is not taking orders for {} right now.", p.id)));
    }
    if p.limit_only && !intent.limit {
        return Err(refuse("limit_only", format!("{} takes limit orders only right now.", p.id)));
    }
    let Some(q) = s.quote else {
        return Err(refuse("no_price", format!("There is no current price for {}.", p.id)));
    };
    if s.now - q.at > STALE_QUOTE || q.bid <= 0.0 || q.ask <= 0.0 || q.ask < q.bid {
        return Err(refuse("stale_price", format!("The price for {} is stale or broken; not trading on it.", p.id)));
    }
    for (limit, count, whose) in [(s.limits.orders_per_hour, s.orders_last_hour, "this strategy"), (s.global.orders_per_hour, s.orders_last_hour_all, "Arbiter")] {
        if limit.is_some_and(|l| count >= l) {
            return Err(refuse("too_many_orders", format!("{whose} has placed {count} orders in the last hour, its limit.")));
        }
    }
    let cid = client_order_id(&intent.cause);
    // The best price on this side for a post-only limit, inside the band around the middle.
    let limit_price = |side: Side| -> Result<Decimal, Refusal> {
        let raw = if side == Side::Buy { q.bid } else { q.ask };
        let price = floor_to(Decimal::from_f64(raw).unwrap_or_default(), p.price_increment);
        let mid = q.mid();
        let off = (raw - mid).abs() / mid * 100.0;
        if off > s.global.price_band_pct.to_f64().unwrap_or(2.0) {
            return Err(refuse("outside_band", format!("The best price is {off:.1}% from the middle: the book is too thin to trade.")));
        }
        Ok(price)
    };
    match intent.side {
        Side::Buy => {
            let Some(want) = intent.quote.filter(|w| *w > Decimal::ZERO) else {
                return Err(refuse("no_amount", "A buy needs an amount to spend."));
            };
            if let Some(max) = s.limits.max_order.filter(|m| want > *m) {
                return Err(refuse("over_max_order", format!("{} is more than the {} allowed per order.", money(want), money(max))));
            }
            if let Some(max) = s.limits.max_position.filter(|m| s.held_cost + want > *m) {
                return Err(refuse("over_max_position", format!("It would hold {} at cost; the limit is {}.", money(s.held_cost + want), money(max))));
            }
            if let Some(max) = s.global.max_exposure.filter(|m| s.exposure + want > *m) {
                return Err(refuse("over_exposure", format!("Arbiter would hold {} across strategies; the limit is {}.", money(s.exposure + want), money(max))));
            }
            if let Some(max) = s.limits.daily_loss.filter(|m| s.loss_today >= *m) {
                return Err(refuse("daily_loss", format!("This strategy is down {} today, its daily limit of {}.", money(s.loss_today), money(max))));
            }
            if let Some(max) = s.global.daily_loss.filter(|m| s.loss_today_all >= *m) {
                return Err(refuse("daily_loss", format!("Arbiter is down {} today, its daily limit of {}.", money(s.loss_today_all), money(max))));
            }
            if let (Some(mins), Some(at)) = (s.limits.cooldown_minutes, s.last_loss_at) {
                let until = at + i64::from(mins) * 60;
                if s.now < until {
                    return Err(refuse("cooldown", format!("Cooling down after a loss for {} more minutes.", (until - s.now + 59) / 60)));
                }
            }
            if intent.limit {
                let price = limit_price(Side::Buy)?;
                let base = floor_to(want / price, p.base_increment);
                if base < p.base_min_size || base <= Decimal::ZERO {
                    return Err(refuse("too_small", format!("{} buys less than the smallest order of {} {}.", money(want), p.base_min_size.normalize(), p.base)));
                }
                Ok(OrderRequest { client_order_id: cid, product: p.id.clone(), side: Side::Buy, quote_size: None, base_size: Some(base), limit_price: Some(price) })
            } else {
                let spend = floor_to(want, p.quote_increment);
                if spend < p.quote_min_size || spend <= Decimal::ZERO {
                    return Err(refuse("too_small", format!("{} is less than the smallest order of {} {}.", money(want), p.quote_min_size.normalize(), p.quote)));
                }
                Ok(OrderRequest { client_order_id: cid, product: p.id.clone(), side: Side::Buy, quote_size: Some(spend), base_size: None, limit_price: None })
            }
        }
        Side::Sell => {
            if s.held_base <= Decimal::ZERO {
                return Err(refuse("nothing_held", format!("This strategy holds no {} to sell.", p.base)));
            }
            let want = intent.base.unwrap_or(s.held_base).min(s.held_base);
            let base = floor_to(want, p.base_increment);
            if base < p.base_min_size || base <= Decimal::ZERO {
                return Err(refuse("too_small", format!("{} {} is less than the smallest order the exchange takes.", want.normalize(), p.base)));
            }
            let price = if intent.limit { Some(limit_price(Side::Sell)?) } else { None };
            Ok(OrderRequest { client_order_id: cid, product: p.id.clone(), side: Side::Sell, quote_size: None, base_size: Some(base), limit_price: price })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product() -> Product {
        Product {
            id: "ETH-CAD".into(),
            base: "ETH".into(),
            quote: "CAD".into(),
            base_increment: Decimal::new(1, 8),
            quote_increment: Decimal::new(1, 2),
            price_increment: Decimal::new(1, 2),
            base_min_size: Decimal::new(1, 8),
            quote_min_size: Decimal::ONE,
            price: Some(3400.0),
            change_24h: None,
            tradable: true,
            limit_only: false,
        }
    }

    fn intent(side: Side) -> Intent {
        Intent { product: "ETH-CAD".into(), side, quote: Some(Decimal::new(50, 0)), base: None, limit: false, source: Source::Rule, cause: "s1:bar1:buy".into() }
    }

    struct Fixture {
        p: Product,
        limits: Limits,
        global: GlobalLimits,
    }

    impl Fixture {
        fn new() -> Fixture {
            Fixture {
                p: product(),
                limits: Limits { max_order: Some(Decimal::new(50, 0)), max_position: Some(Decimal::new(200, 0)), daily_loss: Some(Decimal::new(15, 0)), orders_per_hour: Some(2), cooldown_minutes: Some(60) },
                global: GlobalLimits::default(),
            }
        }
        fn situation(&self) -> Situation<'_> {
            Situation {
                product: Some(&self.p),
                quote: Some(Quote { bid: 3399.5, ask: 3400.5, at: 1000 }),
                now: 1010,
                limits: &self.limits,
                global: &self.global,
                held_base: Decimal::ZERO,
                held_cost: Decimal::ZERO,
                exposure: Decimal::ZERO,
                loss_today: Decimal::ZERO,
                loss_today_all: Decimal::ZERO,
                orders_last_hour: 0,
                orders_last_hour_all: 0,
                last_loss_at: None,
                halted: false,
                all_halted: false,
            }
        }
    }

    fn code(r: Result<OrderRequest, Refusal>) -> String {
        r.err().map(|e| e.code).unwrap_or_default()
    }

    #[test]
    fn a_plain_buy_passes_and_keeps_its_id() {
        let f = Fixture::new();
        let o = check(&intent(Side::Buy), &f.situation()).unwrap();
        assert_eq!(o.quote_size, Some(Decimal::new(50, 0)));
        assert_eq!(o.client_order_id, check(&intent(Side::Buy), &f.situation()).unwrap().client_order_id);
        assert_eq!(o.client_order_id.len(), 36);
    }

    #[test]
    fn it_refuses_rather_than_shrinks() {
        let f = Fixture::new();
        let mut i = intent(Side::Buy);
        i.quote = Some(Decimal::new(51, 0));
        assert_eq!(code(check(&i, &f.situation())), "arbiter.over_max_order");
        let mut s = f.situation();
        s.held_cost = Decimal::new(160, 0);
        assert_eq!(code(check(&intent(Side::Buy), &s)), "arbiter.over_max_position");
        let mut s = f.situation();
        s.loss_today = Decimal::new(15, 0);
        assert_eq!(code(check(&intent(Side::Buy), &s)), "arbiter.daily_loss");
        let mut s = f.situation();
        s.orders_last_hour = 2;
        assert_eq!(code(check(&intent(Side::Buy), &s)), "arbiter.too_many_orders");
        let mut s = f.situation();
        s.last_loss_at = Some(1000);
        assert_eq!(code(check(&intent(Side::Buy), &s)), "arbiter.cooldown");
    }

    #[test]
    fn an_invented_symbol_stops_here() {
        let f = Fixture::new();
        let mut s = f.situation();
        s.product = None;
        let mut i = intent(Side::Buy);
        i.product = "MOON-CAD".into();
        assert_eq!(code(check(&i, &s)), "arbiter.unknown_product");
        assert_eq!(code(check(&i, &f.situation())), "arbiter.unknown_product");
    }

    #[test]
    fn stale_prices_and_halts_refuse() {
        let f = Fixture::new();
        let mut s = f.situation();
        s.now = 1000 + STALE_QUOTE + 1;
        assert_eq!(code(check(&intent(Side::Buy), &s)), "arbiter.stale_price");
        let mut s = f.situation();
        s.halted = true;
        assert_eq!(code(check(&intent(Side::Buy), &s)), "arbiter.halted");
        // The person may still sell what is held after a halt.
        s.held_base = Decimal::new(1, 2);
        let mut sell = intent(Side::Sell);
        sell.source = Source::Person;
        assert!(check(&sell, &s).is_ok());
        sell.source = Source::Agent;
        assert_eq!(code(check(&sell, &s)), "arbiter.halted");
    }

    #[test]
    fn sizes_and_prices_meet_the_increments() {
        let f = Fixture::new();
        let mut i = intent(Side::Buy);
        i.quote = Some(Decimal::new(49999, 3)); // 49.999
        assert_eq!(check(&i, &f.situation()).unwrap().quote_size, Some(Decimal::new(4999, 2)));
        i.limit = true;
        let o = check(&i, &f.situation()).unwrap();
        assert_eq!(o.limit_price, Some(Decimal::new(339950, 2)));
        assert_eq!(o.base_size, Some(floor_to(Decimal::new(49999, 3) / Decimal::new(339950, 2), Decimal::new(1, 8))));
        let mut s = f.situation();
        s.held_base = Decimal::new(123456789, 10);
        let o = check(&intent(Side::Sell), &s).unwrap();
        assert_eq!(o.base_size, Some(Decimal::new(1234567, 8)));
    }

    #[test]
    fn sells_need_something_held() {
        let f = Fixture::new();
        assert_eq!(code(check(&intent(Side::Sell), &f.situation())), "arbiter.nothing_held");
    }
}
