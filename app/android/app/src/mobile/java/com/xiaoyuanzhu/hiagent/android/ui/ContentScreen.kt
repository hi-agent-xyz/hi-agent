package com.xiaoyuanzhu.hiagent.android.ui

import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.xiaoyuanzhu.hiagent.android.AppModel
import com.xiaoyuanzhu.hiagent.android.core.PairingRequest

/**
 * The root. The attached core's face *is* the app, edge to edge; the roster is a
 * place you visit, not the screen you start on.
 */
@Composable
fun ContentScreen(model: AppModel, modifier: Modifier = Modifier) {
    val entries by model.entries.collectAsStateWithLifecycle()
    val selectedId by model.selectedId.collectAsStateWithLifecycle()
    val pairingRequest by model.pairingRequest.collectAsStateWithLifecycle()
    val pairingLinkError by model.pairingLinkError.collectAsStateWithLifecycle()

    var showingRoster by remember { mutableStateOf(false) }

    val current = remember(entries, selectedId) {
        entries.firstOrNull { it.id == selectedId }
            ?: entries.firstOrNull { it.attached }
            ?: entries.firstOrNull()
    }

    LaunchedEffect(Unit) { model.refreshAll() }

    Box(modifier.fillMaxSize()) {
        Crossfade(
            targetState = current?.id,
            animationSpec = tween(350),
            label = "stage",
        ) { id ->
            val entry = entries.firstOrNull { it.id == id }
            if (entry != null) {
                CoreStage(
                    model = model,
                    entry = entry,
                    onOpenRoster = { showingRoster = true },
                )
            } else {
                WelcomeScreen(
                    onScan = { model.requestPairing(PairingRequest.scan()) },
                    onManual = { model.requestPairing(PairingRequest.manual()) },
                )
            }
        }
    }

    if (showingRoster) {
        RosterSheet(
            model = model,
            onDismiss = { showingRoster = false },
            onPair = {
                showingRoster = false
                model.requestPairing(PairingRequest.scan())
            },
        )
    }

    pairingRequest?.let { request ->
        PairCoreSheet(
            model = model,
            request = request,
            onDismiss = { model.requestPairing(null) },
        )
    }

    pairingLinkError?.let { message ->
        AlertDialog(
            onDismissRequest = { model.clearPairingLinkError() },
            confirmButton = {
                TextButton(onClick = { model.clearPairingLinkError() }) { Text("OK") }
            },
            title = { Text("Could not open pairing link") },
            text = { Text(message) },
        )
    }
}

/**
 * First run. One mark, one sentence, one obvious thing to do — and no modal
 * thrown at the reader before they have looked at the screen.
 */
@Composable
private fun WelcomeScreen(
    onScan: () -> Unit,
    onManual: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(modifier.hiCanvas()) {
        Column(
            modifier = Modifier.align(Alignment.Center).padding(Theme.gutter),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(22.dp),
        ) {
            CoreMark(size = 104.dp)
            Text("Hi Agent", style = MaterialTheme.typography.displaySmall)
            Text(
                text = "Pair a core to open the conversation.",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.widthIn(max = 300.dp),
            )
        }

        Column(
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .fillMaxWidth()
                .navigationBarsPadding()
                .padding(horizontal = Theme.gutter, vertical = 20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Button(
                onClick = onScan,
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(14.dp),
            ) {
                Icon(Icons.Rounded.QrCodeScanner, contentDescription = null)
                Text("  Scan pairing code")
            }
            TextButton(onClick = onManual) { Text("Enter details manually") }
        }
    }
}
