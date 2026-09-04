package com.xiaoyuanzhu.hiagent.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.DeleteOutline
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.xiaoyuanzhu.hiagent.android.AppModel
import com.xiaoyuanzhu.hiagent.android.core.RosterEntry

/**
 * The core switcher. A place you drop into from the stage and leave again — so
 * it is a bottom sheet, not a screen you have to navigate back out of.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RosterSheet(
    model: AppModel,
    onDismiss: () -> Unit,
    onPair: () -> Unit,
) {
    val entries by model.entries.collectAsStateWithLifecycle()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = false)

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = MaterialTheme.colorScheme.surface,
        shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp),
    ) {
        Column(Modifier.navigationBarsPadding()) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(start = 20.dp, end = 8.dp, top = 4.dp, bottom = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "Cores",
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.weight(1f),
                )
                IconButton(onClick = { model.refreshAll() }) {
                    Icon(
                        Icons.Rounded.Refresh,
                        contentDescription = "Re-check every core",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            LazyColumn(Modifier.weight(1f, fill = false)) {
                items(entries, key = { it.id }) { entry ->
                    CoreRow(
                        entry = entry,
                        onSelect = {
                            if (!entry.attached) model.attach(entry.id)
                            onDismiss()
                        },
                        onForget = { model.forget(entry.id) },
                    )
                }
            }

            Button(
                onClick = onPair,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp)
                    .padding(bottom = 16.dp),
                shape = RoundedCornerShape(14.dp),
            ) {
                Icon(Icons.Rounded.QrCodeScanner, contentDescription = null)
                Text("  Pair another core")
            }
        }
    }
}

/**
 * One core. Name, address, and — only when it is worth saying — what the last
 * health check found. A healthy roster stays quiet.
 */
@Composable
private fun CoreRow(
    entry: RosterEntry,
    onSelect: () -> Unit,
    onForget: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onSelect)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        StatusDot(state = entry.health)

        Column(Modifier.weight(1f)) {
            Text(
                text = entry.label,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.SemiBold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = if (entry.health.isLive) {
                    entry.displayHost
                } else {
                    "${entry.displayHost} · ${entry.health.title}"
                },
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
            )
        }

        if (entry.attached) {
            Box(Modifier.size(24.dp), contentAlignment = Alignment.Center) {
                Icon(
                    Icons.Rounded.Check,
                    contentDescription = "Current core",
                    tint = Theme.ink,
                    modifier = Modifier.size(18.dp),
                )
            }
        }

        // Forgetting is a button rather than a swipe: a swipe on a row inside a
        // bottom sheet competes with the sheet's own drag-to-dismiss, and loses
        // often enough that the action reads as broken.
        IconButton(onClick = onForget) {
            Icon(
                Icons.Rounded.DeleteOutline,
                contentDescription = "Forget ${entry.label}",
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(20.dp),
            )
        }
    }
}
