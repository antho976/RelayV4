package com.tally.app.ui.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Lan
import androidx.compose.material.icons.rounded.Link
import androidx.compose.material.icons.rounded.Sync
import androidx.compose.material.icons.rounded.SyncProblem
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import com.tally.app.data.Clock
import com.tally.app.data.sync.PairLink
import com.tally.app.data.sync.PcSync
import com.tally.app.data.sync.SyncOutcome
import com.tally.app.data.sync.SyncPrefs
import com.tally.app.data.sync.SyncStore
import com.tally.app.ui.common.Dates
import com.tally.app.ui.common.GROUP_SEAM
import com.tally.app.ui.common.GUTTER
import com.tally.app.ui.common.GlyphBadge
import com.tally.app.ui.common.Group
import com.tally.app.ui.common.GroupFooter
import com.tally.app.ui.common.GroupHeader
import com.tally.app.ui.common.GroupRow
import com.tally.app.ui.common.HeroAction
import com.tally.app.ui.common.Notices
import com.tally.app.ui.common.SecondaryAction
import com.tally.app.ui.nav.AppNav
import com.tally.app.ui.plan.PlanField
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import javax.inject.Inject

@Immutable
data class PcState(
    val today: LocalDate,
    val prefs: SyncPrefs = SyncPrefs(),
    /** Pairing or syncing right now. */
    val busy: Boolean = false,
    /** Pairing, which can wait on a person at the PC; says so while it does. */
    val pairing: Boolean = false,
    /** The pairing's outcome when it did not work, the one red line on the form. */
    val problem: String? = null,
    val loaded: Boolean = false,
)

/**
 * Relay on your PC: pair with the PC by its link or its address and code, then see when the two
 * ledgers last met, sync now, or forget the PC (with Undo, like any removal).
 */
@HiltViewModel
class PcViewModel @Inject constructor(
    private val store: SyncStore,
    private val sync: PcSync,
    private val notices: Notices,
    clock: Clock,
) : ViewModel() {

    private val today = clock.today()

    private data class Ui(val pairing: Boolean = false, val problem: String? = null)

    private val ui = MutableStateFlow(Ui())

    val state: StateFlow<PcState> = combine(store.prefs, sync.busy, ui) { p, busy, u ->
        PcState(today = today, prefs = p, busy = busy || u.pairing, pairing = u.pairing, problem = u.problem, loaded = true)
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), PcState(today = today))

    fun pairWithLink(text: String) {
        val link = PairLink.parse(text)
        if (link == null) {
            ui.update { it.copy(problem = "That is not a Relay pairing link. It starts with relay://pair, as relay remote pair prints it.") }
            return
        }
        pair(link)
    }

    fun pairByHand(address: String, code: String) {
        val problem = when {
            PairLink.address(address) == null -> "That address does not read. Type it as the PC printed it, like 192.168.1.20."
            code.none { it.isLetterOrDigit() } -> "Type the code the PC printed, like ABCD-EFGH."
            else -> null
        }
        if (problem != null) {
            ui.update { it.copy(problem = problem) }
            return
        }
        pair(PairLink.manual(address, code) ?: return)
    }

    private fun pair(link: PairLink) {
        if (state.value.busy) return
        ui.update { Ui(pairing = true) }
        viewModelScope.launch {
            val outcome = sync.pair(link)
            val now = store.current()
            val paired = now.paired
            ui.update {
                Ui(problem = if (outcome is SyncOutcome.Failed && !paired) outcome.reason else null)
            }
            when {
                outcome is SyncOutcome.Done -> notices.show("Paired with ${now.pc?.name ?: "your PC"}. Its ledger now matches this phone")
                // Paired, but the first sync did not finish: the status row says why, and it retries.
                paired -> notices.show("Paired. The first sync did not finish yet")
            }
        }
    }

    fun syncNow() {
        if (state.value.busy) return
        viewModelScope.launch {
            when (val outcome = sync.syncNow()) {
                is SyncOutcome.Done -> notices.show(if (outcome.sent == 0 && outcome.received == 0) "Up to date" else "Synced")
                // Quiet on purpose: the status row carries the reason.
                is SyncOutcome.Failed, SyncOutcome.NotPaired -> Unit
            }
        }
    }

    fun forget() {
        viewModelScope.launch {
            val was = sync.forget()
            val name = was.pc?.name ?: return@launch
            notices.showUndo("Forgot $name") { sync.restore(was) }
        }
    }

    fun clearProblem() = ui.update { it.copy(problem = null) }
}

@Composable
fun PcRoute(nav: AppNav) {
    val viewModel: PcViewModel = hiltViewModel()
    val state by viewModel.state.collectAsStateWithLifecycle()
    PcScreen(
        state,
        PcActions(
            back = nav::back,
            pairWithLink = viewModel::pairWithLink,
            pairByHand = viewModel::pairByHand,
            syncNow = viewModel::syncNow,
            forget = viewModel::forget,
            edited = viewModel::clearProblem,
        ),
    )
}

data class PcActions(
    val back: () -> Unit = {},
    val pairWithLink: (String) -> Unit = {},
    val pairByHand: (address: String, code: String) -> Unit = { _, _ -> },
    val syncNow: () -> Unit = {},
    val forget: () -> Unit = {},
    /** A field changed: an old problem line no longer describes it. */
    val edited: () -> Unit = {},
)

/** "Today, 14:20": when a sync last worked, or null before the first. */
internal fun lastSyncOn(prefs: SyncPrefs, today: LocalDate): String? =
    prefs.lastSuccessAt.takeIf { it > 0L }?.let { backupWhen(it, today) }

/**
 * The page has two states. Not paired: the link the PC prints, or its address and code by hand,
 * and what the first sync will do, said before it runs. Paired: the PC, where the last sync
 * stands, Sync now and Forget this PC.
 */
@Composable
fun PcScreen(state: PcState, actions: PcActions) {
    val p = state.prefs
    val pc = p.pc
    LazyColumn(
        Modifier.fillMaxSize().navigationBarsPadding(),
        contentPadding = PaddingValues(bottom = 32.dp),
        verticalArrangement = Arrangement.spacedBy(28.dp),
    ) {
        item(key = "head") {
            PageHead(
                PC_GROUP,
                onBack = actions.back,
                context = if (pc == null) "One ledger on this phone and your computer" else "Paired with ${pc.name}",
            )
        }
        if (pc == null) {
            pairForm(state, actions)
        } else {
            item(key = "status") {
                val line = syncLine(true, lastSyncOn(p, state.today), p.lastError, p.lastSent, p.lastReceived)
                val pcRow: @Composable (Shape) -> Unit = { shape ->
                    val pairedOn = Dates.short(Instant.ofEpochMilli(pc.pairedAt).atZone(ZoneId.systemDefault()).toLocalDate(), state.today)
                    GroupRow(pc.name, shape, subtitle = "Paired $pairedOn", leading = { GlyphBadge(Icons.Rounded.Computer) })
                }
                val syncRow: @Composable (Shape) -> Unit = { shape ->
                    val tint = if (line.failed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onBackground
                    GroupRow(
                        if (state.busy) "Syncing" else line.title,
                        shape,
                        subtitle = line.detail,
                        leading = {
                            GlyphBadge(
                                if (line.failed) Icons.Rounded.SyncProblem else Icons.Rounded.Sync,
                                tint = tint,
                                fill = if (line.failed) tint.copy(alpha = 0.14f) else MaterialTheme.colorScheme.surfaceContainerHighest,
                            )
                        },
                    )
                }
                Group(
                    rows = listOf(pcRow, syncRow),
                    modifier = Modifier.padding(horizontal = GUTTER),
                    title = "This PC",
                    footer = when {
                        !p.replaced -> FIRST_SYNC_NOTE
                        else -> listOfNotNull(
                            p.note?.let { "$it." },
                            "Tally syncs when you open it, a few seconds after a change, and every half hour while there is a network.",
                        ).joinToString(" ")
                    },
                )
            }
            item(key = "acts") {
                Column(Modifier.padding(horizontal = GUTTER), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    HeroAction(if (state.busy) "Syncing" else "Sync now", actions.syncNow, Modifier.fillMaxWidth(), enabled = !state.busy)
                    SecondaryAction("Forget this PC", actions.forget, Modifier.fillMaxWidth(), destructive = true, enabled = !state.busy)
                    GroupFooter("Forgetting stops the syncing. The PC keeps its copy of the ledger.")
                }
            }
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.pairForm(state: PcState, actions: PcActions) {
    item(key = "link") {
        var link by rememberSaveable { mutableStateOf("") }
        Column(Modifier.padding(horizontal = GUTTER)) {
            GroupHeader("With the pairing link")
            PlanField(
                link,
                { link = it.trim(); actions.edited() },
                placeholder = "Paste the relay://pair link",
                icon = Icons.Rounded.Link,
                keyboardType = KeyboardType.Uri,
            )
            GroupFooter("On the PC, run relay remote pair. It prints this link under its QR code.")
            HeroAction(
                if (state.pairing) "Pairing" else "Pair and sync",
                { actions.pairWithLink(link) },
                Modifier.fillMaxWidth().padding(top = 14.dp),
                enabled = !state.busy && link.isNotBlank(),
            )
        }
    }
    item(key = "manual") {
        var address by rememberSaveable { mutableStateOf("") }
        var code by rememberSaveable { mutableStateOf("") }
        Column(Modifier.padding(horizontal = GUTTER)) {
            GroupHeader("Or by hand")
            Column(verticalArrangement = Arrangement.spacedBy(GROUP_SEAM)) {
                PlanField(
                    address,
                    { address = it; actions.edited() },
                    placeholder = "The PC's address, like 192.168.1.20",
                    icon = Icons.Rounded.Lan,
                    keyboardType = KeyboardType.Uri,
                )
                PlanField(
                    code,
                    { code = it; actions.edited() },
                    placeholder = "The code, like ABCD-EFGH",
                    icon = Icons.Rounded.Key,
                    capitalization = KeyboardCapitalization.Characters,
                )
            }
            GroupFooter("The address and the code are the ones relay remote pair prints.")
            SecondaryAction(
                if (state.pairing) "Pairing" else "Pair by address",
                { actions.pairByHand(address, code) },
                Modifier.fillMaxWidth().padding(top = 14.dp),
                enabled = !state.busy && address.isNotBlank() && code.isNotBlank(),
            )
        }
    }
    item(key = "note") {
        Column(Modifier.padding(horizontal = GUTTER)) {
            val problem = state.problem
            when {
                state.pairing -> GroupFooter("Waiting for the PC. If it asks, approve this phone there.")
                problem != null -> GroupFooter(problem, isError = true)
            }
            GroupFooter(FIRST_SYNC_NOTE)
            GroupFooter("Your ledger goes only to the PC you pair, over your own network. No cloud, no account.")
        }
    }
}
