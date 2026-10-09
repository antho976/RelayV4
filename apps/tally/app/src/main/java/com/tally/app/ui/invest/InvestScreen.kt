package com.tally.app.ui.invest

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.InsertDriveFile
import androidx.compose.material.icons.rounded.AccountBalanceWallet
import androidx.compose.material.icons.rounded.AddCard
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.CurrencyExchange
import androidx.compose.material.icons.rounded.Sell
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.tally.app.ui.accounts.TilePair
import com.tally.app.ui.common.Aside
import com.tally.app.ui.common.Caption
import com.tally.app.ui.common.ChoiceChip
import com.tally.app.ui.common.ChromeButton
import com.tally.app.ui.common.Dates
import com.tally.app.ui.common.DayBars
import com.tally.app.ui.common.EndsRow
import com.tally.app.ui.common.GUTTER
import com.tally.app.ui.common.GlyphBadge
import com.tally.app.ui.common.Group
import com.tally.app.ui.common.GroupRow
import com.tally.app.ui.common.HeroAction
import com.tally.app.ui.common.HeroNumber
import com.tally.app.ui.common.HeroPanel
import com.tally.app.ui.common.LocalMoney
import com.tally.app.ui.common.PANEL_GAP
import com.tally.app.ui.common.PageTitle
import com.tally.app.ui.common.Panel
import com.tally.app.ui.common.PanelHeader
import com.tally.app.ui.common.READING_WIDTH
import com.tally.app.ui.common.RowPill
import com.tally.app.ui.common.SecondaryAction
import com.tally.app.ui.common.StackedBar
import com.tally.app.ui.common.StatChip
import com.tally.app.ui.common.StatTile
import com.tally.app.ui.common.TextAction
import com.tally.app.ui.common.TopBar
import com.tally.app.ui.nav.AppNav
import com.tally.app.ui.theme.categoryColor
import com.tally.core.Copy
import com.tally.core.Registration
import java.time.LocalDate
import java.time.format.TextStyle
import java.util.Locale

@Composable
fun InvestRoute(nav: AppNav) {
    val viewModel: InvestViewModel = hiltViewModel()
    val state by viewModel.state.collectAsStateWithLifecycle()
    LifecycleResumeEffect(viewModel) {
        viewModel.onResume()
        onPauseOrDispose { }
    }
    val actions = remember(nav) {
        InvestActions(
            back = nav::back,
            importFile = { nav.import(INVEST_SOURCE) },
            openAccount = { nav.accountEdit(it) },
            addAccount = { nav.accountEdit(0) },
            setRoom = { nav.roomEdit(it) },
            pc = nav::pc,
        )
    }
    InvestScreen(state, actions)
}

/** Everything the Investments screen can do, as plain lambdas, so it renders in a test with no graph. */
data class InvestActions(
    val back: () -> Unit = {},
    /** The import page, opened on Wealthsimple's investment files. */
    val importFile: () -> Unit = {},
    /** An investment account's editor: its value by hand, its kind. */
    val openAccount: (Long) -> Unit = {},
    val addAccount: () -> Unit = {},
    /** This year's room figure for one registration. */
    val setRoom: (Registration) -> Unit = {},
    val pc: () -> Unit = {},
)

/** One column, capped at the reading width and centred on a tablet, as Home's column is. */
private val LANE = Modifier.widthIn(max = READING_WIDTH)

/**
 * Investments: what the portfolio is worth against what its holdings cost, drawn as one bar; this
 * year's registered room read as a pace to its deadline; the holdings, how they split, what they
 * paid out, the accounts, and how to bring it up to date. With two kinds of account or more, the
 * lens re-reads the whole page for one kind. Every figure carries the day it rests on: the phone
 * has no live quotes, so a page that has gone stale says so.
 */
@Composable
fun InvestScreen(state: InvestState, actions: InvestActions, initialLens: String? = null) {
    var lensKey by rememberSaveable { mutableStateOf(initialLens) }
    val view = remember(state, lensKey) { investView(state.portfolio, state.accounts, lensKey) }
    val panel = LANE.padding(horizontal = GUTTER)
    LazyColumn(
        Modifier.fillMaxSize().navigationBarsPadding(),
        contentPadding = PaddingValues(bottom = 32.dp),
        verticalArrangement = Arrangement.spacedBy(PANEL_GAP),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        item(key = "head") { InvestHead(state, view, actions, LANE) }
        // Until the first read lands every figure is a placeholder; "$0, nothing tracked" over a
        // full portfolio would say something untrue, so only the frame shows.
        if (state.loaded) {
            val problem = state.problem
            when {
                problem != null -> {
                    item(key = "problem") { Aside(problem, panel.fillMaxWidth()) }
                    item(key = "update") { UpdateGroup(actions, panel) }
                }
                view.accounts.isEmpty() -> {
                    item(key = "zero") { ZeroHero(actions, panel) }
                    item(key = "how") { HowToGroup(panel) }
                }
                else -> {
                    if (view.lenses.isNotEmpty()) {
                        item(key = "lens") { LensChips(view) { lensKey = it } }
                    }
                    item(key = "hero") { PortfolioHero(state, view, actions, panel) }
                    // With no holdings on record there is no cost to read a return on; the hero says so once.
                    if (view.hasHoldings) {
                        item(key = "figures") { Figures(view, panel) }
                    }
                    if (view.room.isNotEmpty()) {
                        item(key = "room") { RoomPanel(state, view, actions, panel) }
                    }
                    if (view.holdings.isNotEmpty()) {
                        item(key = "holdings") { HoldingsPanel(view, panel) }
                    }
                    if (view.allocation.size > 1 || view.kinds.isNotEmpty()) {
                        item(key = "allocation") { AllocationPanel(view, panel) }
                    }
                    item(key = "income") { IncomePanel(state, view, panel) }
                    item(key = "accounts") { AccountsPanel(state, view, actions, panel) }
                    item(key = "update") { UpdateGroup(actions, panel) }
                }
            }
        }
    }
}

/** The top bar with the import capsule, then the serif title and how fresh its figures are. */
@Composable
private fun InvestHead(state: InvestState, view: InvestView, actions: InvestActions, modifier: Modifier = Modifier) {
    Column(modifier) {
        TopBar(onBack = actions.back) {
            ChromeButton(Icons.AutoMirrored.Rounded.InsertDriveFile, "Import a Wealthsimple file", actions.importFile)
        }
        PageTitle(
            "Investments",
            Modifier.padding(horizontal = GUTTER).padding(top = 4.dp, bottom = 6.dp),
            context = if (state.loaded && state.problem == null && view.accounts.isNotEmpty()) {
                investContext(view.accounts.size, view.newest, state.today) { Dates.short(it, state.today) }
            } else {
                null
            },
        )
    }
}

/** The kind lens: All, then each kind held. Chips rather than segments, since the kinds are the owner's and can be many. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun LensChips(view: InvestView, onPick: (String?) -> Unit) {
    FlowRow(
        LANE.padding(horizontal = GUTTER).fillMaxWidth().selectableGroup(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        ChoiceChip("All", view.lens == null, role = Role.Tab) { onPick(null) }
        view.lenses.forEach { l -> ChoiceChip(l.label, view.lens?.key == l.key, role = Role.Tab) { onPick(l.key) } }
    }
}

/**
 * The one serif hero: what the accounts are worth, how far above or below what the holdings cost,
 * that margin drawn as the cost-and-gain bar, and chips for what the bar cannot say. A figure older
 * than a month says how old, with the act that refreshes it.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun PortfolioHero(state: InvestState, view: InvestView, actions: InvestActions, modifier: Modifier = Modifier) {
    val money = LocalMoney.current
    val newest = view.newest
    val worth = money.formatWhole(view.worth)
    val line = when {
        !view.hasHoldings -> "Recorded by hand, across " + Copy.plural(view.accounts.size, "account")
        view.byHand > 0L -> costLine(view.gain, view.cost, money) + " · " + money.formatWhole(view.byHand) + " recorded by hand"
        else -> costLine(view.gain, view.cost, money)
    }
    HeroPanel(modifier) {
        PanelHeader(
            view.lens?.label ?: "Portfolio",
            meta = newest?.let { "AS OF " + Dates.short(it, state.today).uppercase() },
        )
        Spacer(Modifier.height(14.dp))
        HeroNumber(worth, description = "Worth $worth. $line")
        Text(line, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (view.hasHoldings) {
            Spacer(Modifier.height(16.dp))
            CostGainBar(view.cost, view.priced, view.byHand)
        } else {
            Spacer(Modifier.height(10.dp))
            Aside("Import a holdings report to see the cost and the gain")
        }
        // What the bar cannot say: cash not yet put to work, and figures read without a day's price or rate.
        if (view.cash > 0L || view.fxEstimated > 0 || view.noPrice > 0) {
            Spacer(Modifier.height(16.dp))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (view.cash > 0L) StatChip(Icons.Rounded.AccountBalanceWallet, money.formatWhole(view.cash) + " in cash, not invested")
                if (view.fxEstimated > 0) {
                    StatChip(Icons.Rounded.CurrencyExchange, Copy.plural(view.fxEstimated, "holding") + " converted without the day's rate")
                }
                if (view.noPrice > 0) StatChip(Icons.Rounded.Sell, Copy.plural(view.noPrice, "holding") + " with no price, shown at cost")
            }
        }
        if (newest != null && isStale(newest, state.today)) {
            Spacer(Modifier.height(10.dp))
            EndsRow(
                start = { Aside(staleLine(newest, state.today)) },
                end = { TextAction("update", actions.importFile, color = MaterialTheme.colorScheme.primary) },
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

/**
 * Two different readings under the hero: the return on cost (said for what it is, not a
 * time-weighted figure), and what went into registered accounts this year against their room;
 * with no registered account, the cash waiting to be invested instead.
 */
@Composable
private fun Figures(view: InvestView, modifier: Modifier = Modifier) {
    val money = LocalMoney.current
    val bps = view.gainBps
    val putIn = view.room.sumOf { it.contributed }
    val room = view.room.mapNotNull { it.room }
    TilePair(
        modifier,
        first = { m ->
            StatTile(
                "Return",
                if (bps == null) "None" else percentText(bps, signed = true),
                m,
                detail = if (bps == null) "No cost on record yet" else "On cost, not time-weighted",
            )
        },
        second = { m ->
            if (view.room.isEmpty()) {
                StatTile("Cash", money.formatWhole(view.cash), m, detail = if (view.cash > 0L) "Held, not invested" else "All of it invested")
            } else {
                StatTile(
                    "Put in",
                    money.formatWhole(putIn),
                    m,
                    detail = if (room.isEmpty()) "This year, into registered accounts" else "This year, of " + money.formatWhole(room.sum()) + " room",
                )
            }
        },
    )
}

/** The signature section: each registration's room this year, read as a pace to its deadline. */
@Composable
private fun RoomPanel(state: InvestState, view: InvestView, actions: InvestActions, modifier: Modifier = Modifier) {
    Panel(modifier) {
        // In RRSP season the RRSP row reads last year, so a year shared by every row is the only one to show.
        PanelHeader("Room", meta = view.room.map { it.year }.distinct().singleOrNull()?.toString())
        view.room.forEach { line -> key(line.registration) { RoomRow(line, state.today) { actions.setRoom(line.registration) } } }
        Spacer(Modifier.height(10.dp))
        Caption("Your room from CRA My Account, less what went in. The tick is an even pace to the deadline.")
    }
}

/** The holdings, largest first: eight, then the rest on request. */
@Composable
private fun HoldingsPanel(view: InvestView, modifier: Modifier = Modifier) {
    var all by rememberSaveable { mutableStateOf(false) }
    val shown = if (all) view.holdings else view.holdings.take(HOLDINGS_SHOWN)
    val more = view.holdings.size - shown.size
    Panel(modifier) {
        PanelHeader("Holdings", meta = view.holdings.size.toString())
        Spacer(Modifier.height(4.dp))
        shown.forEach { h -> key(h.accountId, h.securityId) { HoldingRow(h, shareBps(h.value, view.priced)) } }
        if (more > 0) {
            TextAction("show $more more", { all = true }, Modifier.align(Alignment.End), color = MaterialTheme.colorScheme.primary)
        }
    }
}

/** How the worth splits: by account kind (with every account read), and by kind of security. */
@Composable
private fun AllocationPanel(view: InvestView, modifier: Modifier = Modifier) {
    Panel(modifier) {
        PanelHeader("Allocation")
        if (view.allocation.size > 1) {
            Spacer(Modifier.height(10.dp))
            ShareBlock("By account", view.allocation)
        }
        if (view.kinds.isNotEmpty()) {
            Spacer(Modifier.height(if (view.allocation.size > 1) 20.dp else 10.dp))
            ShareBlock("By type", view.kinds)
        }
    }
}

/**
 * What the holdings paid: twelve months as bars, this one lit, and the latest payouts. Under a
 * lens only the payouts show, since the months are read for every account together.
 */
@Composable
private fun IncomePanel(state: InvestState, view: InvestView, modifier: Modifier = Modifier) {
    val money = LocalMoney.current
    val months = view.incomeByMonth
    val total = view.income12m
    val names = view.accounts.associate { it.id to it.name }
    val hues = view.holdings.associate { it.symbol to categoryColor(kindHue(it.kind)) }
    Panel(modifier) {
        PanelHeader("Income", meta = if (months != null) "12 MONTHS" else "LATEST")
        if (months != null && total != null) {
            Spacer(Modifier.height(6.dp))
            Text(incomeLine(total, money), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (total > 0L && months.isNotEmpty()) {
                Spacer(Modifier.height(14.dp))
                MonthBars(months.map { it.amount }, months.map { monthStart(it) })
            }
        }
        if (view.income.isEmpty()) {
            Spacer(Modifier.height(8.dp))
            Aside("Dividends and interest show here once an activities export with them is imported")
        } else {
            Spacer(Modifier.height(6.dp))
            view.income.take(INCOME_SHOWN).forEach { e ->
                IncomeRow(e, names[e.accountId], e.symbol?.let { hues[it] } ?: categoryColor(null), state.today)
            }
        }
    }
}

/** Twelve months of payouts as the week's bars are drawn: past months in the quiet rung, this one lit. */
@Composable
private fun MonthBars(amounts: List<Long>, starts: List<LocalDate?>) {
    val money = LocalMoney.current
    val locale: Locale = LocalConfiguration.current.locales[0]
    val labels = starts.map { it?.month?.getDisplayName(TextStyle.NARROW_STANDALONE, locale).orEmpty() }
    val description = "Income by month: " + amounts.mapIndexed { i, v ->
        (starts[i]?.let { Dates.monthShort(it) } ?: "") + " " + money.formatWhole(v)
    }.joinToString(", ")
    DayBars(amounts, labels, highlight = amounts.lastIndex, description = description, height = 72.dp)
}

/** Every investment account: its kind, its day, its return; a tap opens it to record a value or set its kind. */
@Composable
private fun AccountsPanel(state: InvestState, view: InvestView, actions: InvestActions, modifier: Modifier = Modifier) {
    Panel(modifier) {
        PanelHeader("Accounts", meta = view.accounts.size.toString())
        Spacer(Modifier.height(4.dp))
        view.accounts.forEach { a -> key(a.id) { InvestAccountRow(a, state.today) { actions.openAccount(a.id) } } }
        Spacer(Modifier.height(6.dp))
        Caption(
            if (view.accounts.any { it.returnBps != null }) {
                "Returns are money-weighted, from the deposits Tally has seen. Tap an account to record a value or set its kind."
            } else {
                "Tap an account to record a value by hand or set its kind."
            },
        )
    }
}

/** How the figures get here: a Wealthsimple file on this phone, Relay on the PC, or an account kept by hand. */
@Composable
private fun UpdateGroup(actions: InvestActions, modifier: Modifier = Modifier) {
    val file: @Composable (Shape) -> Unit = { shape ->
        GroupRow(
            "Import a Wealthsimple file",
            shape,
            subtitle = "A holdings report or an activities export, from Documents on wealthsimple.com",
            leading = { GlyphBadge(Icons.AutoMirrored.Rounded.InsertDriveFile) },
            trailing = { RowPill("Choose file") },
            chevron = false,
            onClick = actions.importFile,
        )
    }
    val pc: @Composable (Shape) -> Unit = { shape ->
        GroupRow(
            "Bring them in on your PC",
            shape,
            subtitle = "Relay walks you through Wealthsimple's files, and they reach this phone with each sync",
            leading = { GlyphBadge(Icons.Rounded.Computer) },
            onClick = actions.pc,
        )
    }
    val add: @Composable (Shape) -> Unit = { shape ->
        GroupRow(
            "Add an account by hand",
            shape,
            subtitle = "For one no file covers: record its value now and then",
            leading = { GlyphBadge(Icons.Rounded.AddCard) },
            onClick = actions.addAccount,
        )
    }
    Group(
        rows = listOf(file, pc, add),
        modifier = modifier,
        title = "Bring it up to date",
        footer = "Files are read on this phone and not kept. Buys, sells and dividends stay out of your spending and income.",
    )
}

/**
 * Honest at zero: the figure is $0 over an empty bar, and the two ways in are the page's acts:
 * a Wealthsimple file, or an account kept by hand.
 */
@Composable
private fun ZeroHero(actions: InvestActions, modifier: Modifier = Modifier) {
    val money = LocalMoney.current
    HeroPanel(modifier) {
        PanelHeader("Portfolio")
        Spacer(Modifier.height(14.dp))
        HeroNumber(money.formatWhole(0L))
        Text("Nothing tracked here yet", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.height(16.dp))
        // An empty bar still draws its track: the shape of what will fill it.
        StackedBar(emptyList(), "Nothing tracked yet", height = 12.dp)
        Spacer(Modifier.height(20.dp))
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            HeroAction("Import a Wealthsimple file", actions.importFile, Modifier.fillMaxWidth())
            SecondaryAction("Add an account by hand", actions.addAccount, Modifier.fillMaxWidth())
        }
    }
}

/** Where Wealthsimple keeps its files, in three steps, each led by its number. */
@Composable
private fun HowToGroup(modifier: Modifier = Modifier) {
    val steps = listOf(
        "Sign in at wealthsimple.com in a browser" to "Wealthsimple generates its documents on the website",
        "Your profile, then Documents, then Generate document" to "Pick Holdings report (CSV), today, and tick every account",
        "Open the file with Tally" to "Share it from your downloads, or choose it with Import. Then do the same with Activities export (CSV)",
    )
    val rows = steps.mapIndexed { i, (title, subtitle) ->
        val row: @Composable (Shape) -> Unit = { shape ->
            GroupRow(
                title,
                shape,
                subtitle = subtitle,
                leading = { SymbolTile((i + 1).toString(), MaterialTheme.colorScheme.onSurfaceVariant) },
            )
        }
        row
    }
    Group(rows = rows, modifier = modifier, title = "How to get the file", footer = "The file is read on this phone and not kept.")
}
