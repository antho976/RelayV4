package com.tally.app.ui.invest

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.TrendingDown
import androidx.compose.material.icons.automirrored.rounded.TrendingUp
import androidx.compose.material.icons.rounded.Payments
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.tally.app.ui.common.AccountBadge
import com.tally.app.ui.common.Dates
import com.tally.app.ui.common.EndsRow
import com.tally.app.ui.common.GlyphBadge
import com.tally.app.ui.common.LegendDot
import com.tally.app.ui.common.LocalMoney
import com.tally.app.ui.common.PaceMeter
import com.tally.app.ui.common.ROW_FOCUS_OUTSET
import com.tally.app.ui.common.StackedBar
import com.tally.app.ui.common.bounceClick
import com.tally.app.ui.theme.categoryColor
import com.tally.core.AccountType
import com.tally.core.IncomeEntry
import com.tally.core.MoneyFormatter
import com.tally.core.PortfolioHolding
import com.tally.core.Registration
import com.tally.core.RoomLine
import java.time.LocalDate

/*
 * The Investments screen's own pieces: the symbol tile, the room row (the pace meter re-read as a
 * tax room), the cost-and-gain bar, and the rows for a holding, a payout and an account. Bare rows
 * on the page, as Home's are; only the room and account rows are taps.
 */

/** A tile's letters stop growing here, so a symbol stays a tile at 200%. */
private const val TILE_MAX_SCALE = 1.3f

/**
 * A security's mark: its symbol in mono letters on a 15% wash of [hue]. The hue is a data series
 * (the kind of security, or an account's kind), so it washes the tile and never colours the
 * letters. A long symbol widens the tile instead of being cut. The tile is a mark beside a name
 * that says the same, so TalkBack skips it.
 */
@Composable
internal fun SymbolTile(symbol: String, hue: Color, modifier: Modifier = Modifier, size: Dp = 44.dp) {
    val density = LocalDensity.current
    CompositionLocalProvider(LocalDensity provides Density(density.density, density.fontScale.coerceAtMost(TILE_MAX_SCALE))) {
        Box(
            modifier
                .clearAndSetSemantics { }
                .sizeIn(minWidth = size, minHeight = size)
                .clip(RoundedCornerShape(12.dp))
                .background(hue.copy(alpha = 0.15f))
                .padding(4.dp),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                tileSymbol(symbol),
                style = if (size < 40.dp) MaterialTheme.typography.labelMedium else MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onBackground,
                textAlign = TextAlign.Center,
            )
        }
    }
}

/** An account kind's mark: its letters on its own hue, or the account glyph for a kind without a short name. */
@Composable
internal fun RegistrationTile(registration: Registration?, size: Dp = 44.dp) {
    val code = registrationCode(registration)
    if (code == null) {
        AccountBadge(AccountType.INVESTMENT, size = size)
    } else {
        SymbolTile(code, categoryColor(registrationHue(registration)), size = size)
    }
}

/**
 * One registration's room, Home's envelope row re-read as a tax room. The track is this year's
 * room, the fill what went in, and the tick where an even pace to use it by its deadline stands
 * today; past the room the meter stretches, the room becomes a notch and the excess draws in red,
 * because over-contributing is a real penalty. The whole row opens the room's editor.
 */
@Composable
internal fun RoomRow(line: RoomLine, today: LocalDate, onClick: () -> Unit) {
    val money = LocalMoney.current
    val label = registrationLabel(line.registration)
    val room = line.room
    val pace = if (room != null && room > 0L) roomPaceFraction(line.registration, line.year, today) else null
    val filled = if (room != null && room > 0L) (line.contributed.toDouble() / room).toFloat() else 0f
    val text = roomText(line, today, money) { Dates.short(it, today) }
    val reading = roomReading(line, money)
    Column(
        Modifier
            .fillMaxWidth()
            .bounceClick(label = "Set the $label room", focusOutset = ROW_FOCUS_OUTSET, onClick = onClick)
            .heightIn(min = 56.dp)
            .padding(top = 12.dp, bottom = 4.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            RegistrationTile(line.registration, size = 36.dp)
            EndsRow(
                start = {
                    Column {
                        Text(label, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onBackground)
                        Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                },
                end = { Text(reading, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onBackground) },
                modifier = Modifier.weight(1f),
            )
        }
        Spacer(Modifier.height(10.dp))
        PaceMeter(filled, pace, roomDescription(line, pace, money), height = 6.dp)
    }
}

/**
 * What the holdings cost against what they are worth, drawn: the cost in the accent's quiet rung
 * and the gain in the full accent, so the margin is a length, not a claim. Below cost the bar is
 * the cost, worth fills it and the shortfall is a red length, a true state. Values recorded by
 * hand close the bar in muted, so it adds up to the hero's figure. No tick: a share has no pace.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun CostGainBar(cost: Long, priced: Long, byHand: Long, modifier: Modifier = Modifier) {
    val money = LocalMoney.current
    val costColor = MaterialTheme.colorScheme.secondary
    val gainColor = MaterialTheme.colorScheme.primary
    val lossColor = MaterialTheme.colorScheme.error
    val handColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f)
    val gain = priced - cost
    val segments = remember(cost, priced, byHand, costColor, gainColor, lossColor, handColor) {
        buildList<Pair<Float, Color>> {
            if (gain >= 0L) {
                if (cost > 0L) add(cost.toFloat() to costColor)
                if (gain > 0L) add(gain.toFloat() to gainColor)
            } else {
                if (priced > 0L) add(priced.toFloat() to costColor)
                add((-gain).toFloat() to lossColor)
            }
            if (byHand > 0L) add(byHand.toFloat() to handColor)
        }
    }
    val description = buildList {
        if (gain >= 0L) {
            add("Cost " + money.formatWhole(cost))
            add("gain " + money.formatWhole(gain))
        } else {
            add("Worth " + money.formatWhole(priced))
            add(money.formatWhole(-gain) + " below cost")
        }
        if (byHand > 0L) add(money.formatWhole(byHand) + " valued by hand")
    }.joinToString(", ")
    Column(modifier.fillMaxWidth()) {
        StackedBar(segments, description, height = 12.dp)
        Spacer(Modifier.height(10.dp))
        FlowRow(
            Modifier.fillMaxWidth().clearAndSetSemantics { },
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            if (gain >= 0L) {
                LegendDot(costColor, "Cost " + money.formatWhole(cost))
                LegendDot(gainColor, "Gain " + money.formatWhole(gain))
            } else {
                LegendDot(costColor, "Worth " + money.formatWhole(priced))
                LegendDot(lossColor, "Below cost " + money.formatWhole(-gain))
            }
            if (byHand > 0L) LegendDot(handColor, "By hand " + money.formatWhole(byHand))
        }
    }
}

/**
 * One holding, bare on the page as a ledger row is: its symbol's tile, its name over units,
 * account and weight, its value over the gain. A passive row: nothing opens from it yet.
 */
@Composable
internal fun HoldingRow(h: PortfolioHolding, weightBps: Long) {
    val money = LocalMoney.current
    val title = h.name.ifBlank { tileSymbol(h.symbol) }
    val value = money.formatWhole(h.value)
    Row(
        Modifier
            .fillMaxWidth()
            .semantics(mergeDescendants = true) { contentDescription = holdingDescription(h, money) }
            .heightIn(min = 64.dp)
            .padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        SymbolTile(h.symbol, categoryColor(kindHue(h.kind)))
        EndsRow(
            start = {
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(title, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onBackground)
                    Text(holdingLine(h, weightBps), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            },
            end = {
                Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(value, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onBackground, textAlign = TextAlign.End)
                    GainLine(h, money)
                }
            },
            modifier = Modifier.weight(1f).clearAndSetSemantics { },
            gap = 14.dp,
        )
    }
}

/**
 * A holding's gain: a small arrow is the mark (green up, red down), the figures beside it stay
 * muted and carry their own sign, so colour is never the only cue and never the text.
 */
@Composable
private fun GainLine(h: PortfolioHolding, money: MoneyFormatter) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        if (!h.noPrice && h.gain != 0L) {
            Icon(
                if (h.gain > 0L) Icons.AutoMirrored.Rounded.TrendingUp else Icons.AutoMirrored.Rounded.TrendingDown,
                contentDescription = null,
                tint = if (h.gain > 0L) MaterialTheme.colorScheme.tertiary else MaterialTheme.colorScheme.error,
                modifier = Modifier.size(14.dp),
            )
        }
        Text(
            if (h.noPrice) "No price yet, at cost" else gainText(h.gain, h.gainBps, money),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** One payout: the symbol it came from (or the money glyph for interest), its type, day and account, and the amount. */
@Composable
internal fun IncomeRow(e: IncomeEntry, account: String?, hue: Color, today: LocalDate) {
    val money = LocalMoney.current
    val symbol = e.symbol
    val title = if (symbol != null) tileSymbol(symbol) else incomeTypeLabel(e.type)
    val meta = listOfNotNull(
        incomeTypeLabel(e.type).takeIf { symbol != null },
        parseDay(e.date)?.let { Dates.short(it, today) },
        account,
    ).joinToString(" · ")
    val amount = money.formatSigned(e.amount)
    Row(
        Modifier
            .fillMaxWidth()
            .semantics(mergeDescendants = true) { contentDescription = "$title, $amount, $meta" }
            .heightIn(min = 56.dp)
            .padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        if (symbol != null) SymbolTile(symbol, hue, size = 36.dp) else GlyphBadge(Icons.Rounded.Payments, size = 36.dp)
        EndsRow(
            start = {
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(title, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onBackground)
                    Text(meta, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            },
            end = {
                Text(amount, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onBackground, textAlign = TextAlign.End)
            },
            modifier = Modifier.weight(1f).clearAndSetSemantics { },
            gap = 14.dp,
        )
    }
}

/** One investment account: its kind's tile, its name over kind, day, holdings and return, and its worth. Opens its editor. */
@Composable
internal fun InvestAccountRow(a: InvestLine, today: LocalDate, onClick: () -> Unit) {
    val money = LocalMoney.current
    val meta = accountMeta(a) { Dates.short(it, today) }
    val worth = money.formatWhole(a.worth)
    Row(
        Modifier
            .fillMaxWidth()
            .bounceClick(label = "Edit ${a.name}", focusOutset = ROW_FOCUS_OUTSET, onClick = onClick)
            .semantics(mergeDescendants = true) { contentDescription = "${a.name}, $worth, $meta" }
            .heightIn(min = 64.dp)
            .padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        RegistrationTile(a.registration)
        EndsRow(
            start = {
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(a.name, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onBackground)
                    Text(meta, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            },
            end = {
                Text(worth, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onBackground, textAlign = TextAlign.End)
            },
            modifier = Modifier.weight(1f).clearAndSetSemantics { },
            gap = 14.dp,
        )
    }
}

/** A whole split into shares: a stacked bar in the series' hues, then a legend row per share with its value and percent. */
@Composable
internal fun ShareBlock(title: String, shares: List<ShareLine>, modifier: Modifier = Modifier) {
    val money = LocalMoney.current
    val hues = shares.map { categoryColor(it.hue) }
    val description = "$title: " + shares.joinToString(", ") { it.label + " " + money.formatWhole(it.value) + ", " + sharePercent(it.shareBps) }
    Column(modifier.fillMaxWidth()) {
        Text(title, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.height(8.dp))
        StackedBar(shares.mapIndexed { i, s -> s.value.toFloat() to hues[i] }, description, height = 12.dp)
        Spacer(Modifier.height(6.dp))
        shares.forEachIndexed { i, s ->
            EndsRow(
                start = { LegendDot(hues[i], s.label) },
                end = {
                    Text(
                        money.formatWhole(s.value) + " · " + sharePercent(s.shareBps),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                },
                modifier = Modifier.fillMaxWidth().heightIn(min = 28.dp).clearAndSetSemantics { },
            )
        }
    }
}
