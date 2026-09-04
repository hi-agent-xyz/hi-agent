package com.xiaoyuanzhu.hiagent.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.xiaoyuanzhu.hiagent.android.AppModel
import com.xiaoyuanzhu.hiagent.android.core.RosterEntry

/**
 * Which cores this television knows, and which one it is showing.
 *
 * A full screen rather than the handset's bottom sheet: a sheet is dragged away,
 * and there is nothing on a remote to drag with. Back is the way out, here and
 * everywhere else in this shell.
 *
 * Each core is a row of two controls so that left and right mean something on it
 * — attach, and forget. Forgetting takes no confirmation step for the same reason
 * it takes none on the handset: it deletes this device's credential and nothing
 * of the core's, and pairing again is the undo.
 */
@Composable
fun TvRosterScreen(
    model: AppModel,
    onDismiss: () -> Unit,
    onPair: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val entries by model.entries.collectAsStateWithLifecycle()
    val selectedId by model.selectedId.collectAsStateWithLifecycle()

    val first = remember { FocusRequester() }
    LaunchedEffect(Unit) {
        model.refreshAll()
        runCatching { first.requestFocus() }
    }

    BackHandler { onDismiss() }

    Box(modifier.hiCanvas(), contentAlignment = Alignment.Center) {
        Column(
            modifier = Modifier.overscan().widthIn(max = 760.dp),
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            Text("Cores", style = MaterialTheme.typography.headlineMedium)

            LazyColumn(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                items(entries, key = { it.id }) { entry ->
                    TvRosterRow(
                        entry = entry,
                        attached = entry.id == selectedId,
                        focusRequester = if (entry.id == entries.firstOrNull()?.id) first else null,
                        onAttach = {
                            model.attach(entry.id)
                            onDismiss()
                        },
                        onForget = { model.forget(entry.id) },
                    )
                }
            }

            TvButton(
                text = "Pair another core",
                leading = Icons.Rounded.Add,
                onClick = onPair,
                focusRequester = if (entries.isEmpty()) first else null,
            )
        }
    }
}

@Composable
private fun TvRosterRow(
    entry: RosterEntry,
    attached: Boolean,
    focusRequester: FocusRequester?,
    onAttach: () -> Unit,
    onForget: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        TvButton(
            text = if (attached) "${entry.label}  ·  showing" else entry.label,
            onClick = onAttach,
            focusRequester = focusRequester,
            modifier = Modifier.weight(1f),
        )
        StatusDot(state = entry.health, diameter = 12.dp)
        TvButton(text = "Forget", onClick = onForget)
    }

    Text(
        text = entry.baseUrl,
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(start = 20.dp),
    )
}
