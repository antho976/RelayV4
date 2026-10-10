package com.tally.app.ui.accounts

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.tally.app.ui.common.Aside
import com.tally.app.ui.common.FIGURE_GAP
import com.tally.app.ui.common.GlyphBadge
import com.tally.app.ui.common.PanelHeader
import com.tally.app.ui.common.ROW_PAD
import com.tally.app.ui.common.slab

/*
 * Pieces the Accounts and Categories screens share: the lit panel's opening row, a filled text
 * field that sits in a slab group, the zero-state line and a pair of stat tiles. Internal so the
 * categories package can reach them. The serif figure is common's HeroNumber.
 */

/**
 * The lit panel's opening row: glyph tile, the mono label, an optional reading at the end. It is
 * the kit's [PanelHeader], so a long reading drops under the label instead of breaking it.
 */
@Composable
internal fun HeroHead(
    label: String,
    modifier: Modifier = Modifier,
    end: String? = null,
    tint: Color = MaterialTheme.colorScheme.primary,
) {
    PanelHeader(label, modifier, meta = end, tint = tint)
}

/**
 * A filled text field drawn as one member of a slab group: its glyph, the text, an optional
 * suffix. Done closes the keyboard. [isError] draws the error edge; the reason is the group's
 * footer, never inside the field.
 */
@Composable
internal fun FieldRow(
    value: String,
    onChange: (String) -> Unit,
    label: String,
    icon: ImageVector,
    shape: Shape,
    modifier: Modifier = Modifier,
    keyboardType: KeyboardType = KeyboardType.Text,
    capitalization: KeyboardCapitalization = KeyboardCapitalization.None,
    suffix: String? = null,
    isError: Boolean = false,
) {
    val focus = LocalFocusManager.current
    val edge = MaterialTheme.colorScheme.error
    BasicTextField(
        value = value,
        onValueChange = onChange,
        modifier = modifier
            .fillMaxWidth()
            .semantics { contentDescription = label },
        singleLine = true,
        textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onBackground),
        cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
        keyboardOptions = KeyboardOptions(
            capitalization = capitalization,
            keyboardType = keyboardType,
            imeAction = ImeAction.Done,
        ),
        keyboardActions = KeyboardActions(onDone = { focus.clearFocus() }),
        decorationBox = { inner ->
            Row(
                Modifier
                    .slab(shape)
                    .then(if (isError) Modifier.border(1.dp, edge, shape) else Modifier)
                    .heightIn(min = 56.dp)
                    .padding(horizontal = ROW_PAD, vertical = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                GlyphBadge(icon)
                Box(Modifier.weight(1f)) {
                    if (value.isEmpty()) {
                        Text(label, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    inner()
                }
                if (suffix != null) {
                    Text(suffix, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        },
    )
}

/** A zero state inside a section: one quiet line. */
@Composable
internal fun EmptyNote(text: String, modifier: Modifier = Modifier) {
    Aside(text, modifier.fillMaxWidth().padding(vertical = 6.dp))
}

/** Two stat tiles side by side; stacked past 1.5x font so their figures never wrap mid-number. */
@Composable
internal fun TilePair(
    modifier: Modifier = Modifier,
    first: @Composable (Modifier) -> Unit,
    second: @Composable (Modifier) -> Unit,
) {
    if (LocalDensity.current.fontScale > 1.5f) {
        Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(FIGURE_GAP)) {
            first(Modifier.fillMaxWidth())
            second(Modifier.fillMaxWidth())
        }
    } else {
        Row(modifier.fillMaxWidth().height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(FIGURE_GAP)) {
            first(Modifier.weight(1f).fillMaxHeight())
            second(Modifier.weight(1f).fillMaxHeight())
        }
    }
}
