package com.xiaoyuanzhu.hiagent.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.xiaoyuanzhu.hiagent.android.AppModel
import com.xiaoyuanzhu.hiagent.android.core.AddAgentRequest

/**
 * The root, on a television.
 *
 * The same arrangement the handset has — the attached core's face *is* the app,
 * and the roster is somewhere you go — reached with a remote instead of a thumb.
 * One screen is up at a time and Back is what leaves it, because that is the only
 * navigation a TV remote has; there is no sheet to drag away and no corner to tap
 * outside of.
 */
@Composable
fun TvContentScreen(model: AppModel, modifier: Modifier = Modifier) {
    val entries by model.entries.collectAsStateWithLifecycle()
    val selectedId by model.selectedId.collectAsStateWithLifecycle()
    val addRequest by model.addRequest.collectAsStateWithLifecycle()

    var showingRoster by remember { mutableStateOf(false) }

    val current = remember(entries, selectedId) {
        entries.firstOrNull { it.id == selectedId }
            ?: entries.firstOrNull { it.attached }
            ?: entries.firstOrNull()
    }

    LaunchedEffect(Unit) { model.refreshAll() }

    Box(modifier.fillMaxSize()) {
        val request = addRequest
        when {
            request != null -> TvAddAgentScreen(
                model = model,
                request = request,
                onDismiss = { model.requestAdd(null) },
            )

            showingRoster -> TvRosterScreen(
                model = model,
                onDismiss = { showingRoster = false },
                onAdd = {
                    showingRoster = false
                    model.requestAdd(AddAgentRequest.ask())
                },
            )

            current != null -> TvCoreStage(
                model = model,
                entry = current,
                onOpenRoster = { showingRoster = true },
            )

            else -> TvWelcomeScreen(
                onAdd = { model.requestAdd(AddAgentRequest.ask()) },
            )
        }
    }
}

/**
 * First run. The one thing to do is focused when the screen appears, so the
 * remote's first press is the useful one rather than a hunt for where focus
 * started.
 */
@Composable
private fun TvWelcomeScreen(onAdd: () -> Unit, modifier: Modifier = Modifier) {
    val pairButton = remember { FocusRequester() }
    LaunchedEffect(Unit) { pairButton.requestFocus() }

    Box(modifier.hiCanvas(), contentAlignment = Alignment.Center) {
        Column(
            modifier = Modifier.overscan().padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(24.dp),
        ) {
            CoreMark(size = 132.dp)
            Text("Hi Agent", style = MaterialTheme.typography.displayMedium)
            Text(
                text = "Add your agent to put it on this screen.",
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.widthIn(max = 520.dp),
            )
            TvButton(
                text = "Add a remote hi-agent",
                onClick = onAdd,
                focusRequester = pairButton,
            )
        }
    }
}
