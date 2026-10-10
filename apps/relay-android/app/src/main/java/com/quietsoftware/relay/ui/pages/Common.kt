package com.quietsoftware.relay.ui.pages

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import com.quietsoftware.relay.ui.kit.Gap
import com.quietsoftware.relay.ui.kit.Radii
import com.quietsoftware.relay.ui.kit.Slab
import com.quietsoftware.relay.ui.kit.T
import com.quietsoftware.relay.ui.theme.Relay

/** A short question with its keys, over the screen: Save or Discard, and the like. */
@Composable
fun Ask(title: String, body: String?, onDismiss: () -> Unit, keys: @Composable RowScope.() -> Unit) {
    Dialog(onDismissRequest = onDismiss) {
        Slab(
            Modifier.widthIn(max = 420.dp).fillMaxWidth(),
            padding = PaddingValues(18.dp),
            color = Relay.colors.console,
            edge = Relay.colors.strong,
            shape = Radii.popover,
        ) {
            T(title, style = Relay.type.title, color = Relay.colors.ink)
            body?.let {
                Gap(6.dp)
                T(it, style = Relay.type.ui, color = Relay.colors.ink2)
            }
            Gap(16.dp)
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End), verticalAlignment = Alignment.CenterVertically, content = keys)
        }
    }
}

/** A screen's body: a column of at most 720dp, centred, scrolling, clear of the keyboard and the nav bar. */
@Composable
fun ColumnScope.ScrollBody(content: @Composable ColumnScope.() -> Unit) {
    val scroll = rememberScrollState()
    Column(
        Modifier
            .weight(1f)
            .fillMaxWidth()
            .navigationBarsPadding()
            .verticalScroll(scroll)
            .imePadding(),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Column(Modifier.widthIn(max = 720.dp).fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), content = content)
    }
}
