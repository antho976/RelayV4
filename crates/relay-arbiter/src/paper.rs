//! Paper fills: what an order would have done on the real book, without sending it. Coinbase has
//! no paper trading (its sandbox answers with canned replies), so Arbiter simulates on live
//! prices: a market order takes the other side's best price plus slippage and pays the taker fee;
//! a post-only limit rests until the other side trades through it, then fills at its price and
//! pays the maker fee. Fills are kept in the same book as live ones, so paper and live share
//! every line above the venue.

use crate::model::{ExchangeFill, ExchangeOrder, Fees, OrderRequest, OrderStatus, Quote, Side};
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;

/// Fills an order against `q` if it would fill now: a market order always, a limit only when the
/// price has crossed it. `None` leaves a limit resting.
pub fn try_fill(o: &OrderRequest, q: &Quote, fees: &Fees, slippage_pct: Decimal, at: i64) -> Option<(ExchangeOrder, ExchangeFill)> {
    let slip = slippage_pct / Decimal::ONE_HUNDRED;
    let (price, rate) = match (o.limit_price, o.side) {
        (None, Side::Buy) => (Decimal::from_f64(q.ask)? * (Decimal::ONE + slip), fees.taker),
        (None, Side::Sell) => (Decimal::from_f64(q.bid)? * (Decimal::ONE - slip), fees.taker),
        // A resting bid fills once someone sells at or below it.
        (Some(limit), Side::Buy) if Decimal::from_f64(q.ask)? <= limit => (limit, fees.maker),
        (Some(limit), Side::Sell) if Decimal::from_f64(q.bid)? >= limit => (limit, fees.maker),
        _ => return None,
    };
    if price <= Decimal::ZERO {
        return None;
    }
    let (size, fee) = match (o.quote_size, o.base_size) {
        // Spend the quote amount, the fee included in it, as the exchange does.
        (Some(quote), _) => {
            let fee = quote * rate;
            (((quote - fee) / price).round_dp(8), fee.round_dp(8))
        }
        (None, Some(base)) => (base, (base * price * rate).round_dp(8)),
        (None, None) => return None,
    };
    let order_id = format!("paper-{}", o.client_order_id);
    let fill = ExchangeFill { trade_id: format!("{order_id}-1"), order_id: order_id.clone(), price: price.round_dp(8), size, fee, at };
    let order = ExchangeOrder {
        order_id,
        client_order_id: o.client_order_id.clone(),
        status: OrderStatus::Filled,
        filled_size: size,
        average_filled_price: fill.price,
        total_fees: fee,
        reason: None,
    };
    Some((order, fill))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fees() -> Fees {
        Fees { maker: Decimal::new(4, 3), taker: Decimal::new(6, 3), tier: None }
    }

    fn q() -> Quote {
        Quote { bid: 99.0, ask: 101.0, at: 0 }
    }

    #[test]
    fn a_market_buy_pays_the_ask_slippage_and_taker_fee() {
        let o = OrderRequest { client_order_id: "c".into(), product: "ETH-CAD".into(), side: Side::Buy, quote_size: Some(Decimal::new(100, 0)), base_size: None, limit_price: None };
        let (order, fill) = try_fill(&o, &q(), &fees(), Decimal::ZERO, 5).unwrap();
        assert_eq!(fill.price, Decimal::new(101, 0));
        assert_eq!(fill.fee, Decimal::new(6, 1));
        assert_eq!(fill.size, ((Decimal::new(994, 1)) / Decimal::new(101, 0)).round_dp(8));
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn a_limit_rests_until_the_price_crosses_it() {
        let o = OrderRequest { client_order_id: "c".into(), product: "ETH-CAD".into(), side: Side::Buy, quote_size: None, base_size: Some(Decimal::ONE), limit_price: Some(Decimal::new(99, 0)) };
        assert!(try_fill(&o, &q(), &fees(), Decimal::ZERO, 0).is_none());
        let (_, fill) = try_fill(&o, &Quote { bid: 98.0, ask: 99.0, at: 0 }, &fees(), Decimal::ZERO, 0).unwrap();
        assert_eq!(fill.price, Decimal::new(99, 0));
        assert_eq!(fill.fee, Decimal::new(396, 3));
    }
}
