package com.tally.app.ui.invest

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.DeleteOutline
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.tally.app.ui.common.Caption
import com.tally.app.ui.common.ChoiceChip
import com.tally.app.ui.common.ChromeButton
import com.tally.app.ui.common.Dates
import com.tally.app.ui.common.GUTTER
import com.tally.app.ui.common.HeroAction
import com.tally.app.ui.common.HeroNumber
import com.tally.app.ui.common.HeroPanel
import com.tally.app.ui.common.LocalMoney
import com.tally.app.ui.common.PANEL_GAP
import com.tally.app.ui.common.PaceMeter
import com.tally.app.ui.common.PageTitle
import com.tally.app.ui.common.PanelHeader
import com.tally.app.ui.common.StatChip
import com.tally.app.ui.common.TopBar
import com.tally.app.ui.nav.AppNav
import com.tally.app.ui.plan.PadKey
import com.tally.app.ui.plan.PlanKeypad
import com.tally.core.MoneyFormatter
import com.tally.core.Registration

@Composable
fun RoomEditRoute(nav: AppNav) {
    val viewModel: RoomEditViewModel = hiltViewModel()
    val state by viewModel.state.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.done.collect { nav.back() } }
    val actions = remember(viewModel, nav) {
        RoomEditActions(
            back = nav::back,
            remove = viewModel::remove,
            press = viewModel::press,
            fill = viewModel::fill,
            save = viewModel::save,
        )
    }
    RoomEditScreen(state, actions)
}

/** Everything the room editor can do, as plain lambdas. */
data class RoomEditActions(
    val back: () -> Unit = {},
    val remove: () -> Unit = {},
    val press: (PadKey) -> Unit = {},
    val fill: (Long) -> Unit = {},
    val save: () -> Unit = {},
)

/**
 * The room editor, the budget editor's shape: the room being typed as the one figure, with what
 * already went in this year read against it on the room meter, a fill from the year's limit, where
 * CRA keeps the real figure, then the keypad and the save under the thumb.
 */
@Composable
fun RoomEditScreen(state: RoomEditState, actions: RoomEditActions) {
    val money = LocalMoney.current
    val label = registrationLabel(state.registration)
    Column(Modifier.fillMaxSize().navigationBarsPadding()) {
        TopBar(onBack = actions.back) {
            if (state.existing != null) ChromeButton(Icons.Rounded.DeleteOutline, "Remove the $label room", actions.remove)
        }
        Column(
            Modifier
                .weight(1f)
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(start = GUTTER, end = GUTTER, top = 4.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(PANEL_GAP),
        ) {
            PageTitle("$label room", context = "For ${state.year} · until " + Dates.short(state.deadline, state.today))
            RoomHero(state)
            if (state.loaded) QuickFill(state, actions)
            Caption(roomSource(state.registration))
        }
        Column(
            Modifier
                .fillMaxWidth()
                .padding(start = GUTTER, end = GUTTER, top = 10.dp, bottom = 12.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            PlanKeypad(showDecimal = money.fractionDigits > 0, separator = money.decimalSeparator, onKey = actions.press)
            HeroAction("Save room", actions.save, Modifier.fillMaxWidth(), enabled = state.loaded && state.input.minor > 0L)
        }
    }
}

/**
 * The room being typed, with this year so far read against it: the meter's fill is what went in,
 * its tick an even pace to the deadline, so the new figure's meaning shows at once.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun RoomHero(state: RoomEditState, modifier: Modifier = Modifier) {
    val money = LocalMoney.current
    val minor = state.input.minor
    val formatted = money.format(minor)
    val label = registrationLabel(state.registration)
    val pace = roomPaceFraction(state.registration, state.year, state.today)
    val went = money.formatWhole(state.contributed)
    val line = when {
        minor <= 0L -> "$went in so far for ${state.year}, with no room to read it against"
        state.contributed > minor -> money.formatWhole(state.contributed - minor) + " over this room already"
        else -> money.formatWhole(minor - state.contributed) + " left at this room, after $went in"
    }
    val monthly = monthlyToFill(minor - state.contributed, state.today, state.deadline, MoneyFormatter.pow10(money.fractionDigits))
    val existing = state.existing
    HeroPanel(modifier) {
        PanelHeader("Room for ${state.year}", meta = money.currency.currencyCode)
        Spacer(Modifier.height(10.dp))
        HeroNumber(
            formatted,
            color = if (minor > 0L) MaterialTheme.colorScheme.onBackground else MaterialTheme.colorScheme.onSurfaceVariant,
            description = "$label room, $formatted",
            live = true,
        )
        Text(line, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.height(14.dp))
        PaceMeter(
            if (minor > 0L) (state.contributed.toDouble() / minor).toFloat() else 0f,
            if (minor > 0L) pace else null,
            if (minor > 0L) {
                "$went in of ${money.formatWhole(minor)}. An even pace would be ${money.formatWhole(Math.round(minor * pace.toDouble()))} by today."
            } else {
                "No room typed yet. $went in for ${state.year}."
            },
            height = 14.dp,
        )
        if ((minor > 0L && monthly != null) || (existing != null && existing != minor)) {
            Spacer(Modifier.height(14.dp))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (minor > 0L && monthly != null) {
                    StatChip(Icons.Rounded.Speed, money.formatWhole(monthly) + " a month uses it by " + Dates.short(state.deadline, state.today))
                }
                if (existing != null && existing != minor) StatChip(Icons.Rounded.History, "Was " + money.formatWhole(existing))
            }
        }
    }
}

/**
 * Fills: the year's limit for a TFSA or an FHSA (an RRSP's dollar limit is a ceiling, not anyone's
 * room), or the figure set before.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun QuickFill(state: RoomEditState, actions: RoomEditActions) {
    val money = LocalMoney.current
    val minor = state.input.minor
    val limit = state.limit.takeIf { state.registration != Registration.RRSP }
    val existing = state.existing?.takeIf { it != limit }
    if (limit == null && existing == null) return
    Column(Modifier.fillMaxWidth()) {
        Text("Quick fill", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.height(4.dp))
        FlowRow(Modifier.fillMaxWidth().selectableGroup(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (limit != null) {
                ChoiceChip("The ${state.year} limit · " + money.formatWhole(limit), minor == limit) { actions.fill(limit) }
            }
            if (existing != null) {
                ChoiceChip("Current · " + money.formatWhole(existing), minor == existing) { actions.fill(existing) }
            }
        }
    }
}
