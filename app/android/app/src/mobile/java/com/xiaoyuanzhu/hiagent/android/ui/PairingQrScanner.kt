package com.xiaoyuanzhu.hiagent.android.ui

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CameraAlt
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.ContextCompat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import com.xiaoyuanzhu.hiagent.android.core.PairingLinkException
import com.xiaoyuanzhu.hiagent.android.core.PairingRequest
import java.util.concurrent.Executors

private enum class CameraState { CHECKING, READY, DENIED }

/**
 * Scanning the core's pairing QR.
 *
 * CameraX frames decoded by ZXing, deliberately **not** ML Kit: ML Kit's barcode
 * scanner is delivered through Google Play services, which is precisely the
 * dependency that does not exist on many of the handsets this app is for. ZXing
 * is a plain JAR, and a QR at arm's length is not a hard decode.
 */
@Composable
fun PairingQrScanner(
    onDismiss: () -> Unit,
    onScan: (PairingRequest) -> Unit,
) {
    val context = LocalContext.current
    var cameraState by remember {
        mutableStateOf(
            if (hasCameraPermission(context)) CameraState.READY else CameraState.CHECKING,
        )
    }
    var scanError by remember { mutableStateOf<String?>(null) }

    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        cameraState = if (granted) CameraState.READY else CameraState.DENIED
    }

    LaunchedEffect(Unit) {
        if (cameraState == CameraState.CHECKING) {
            permissionLauncher.launch(Manifest.permission.CAMERA)
        }
    }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(Modifier.fillMaxSize().background(Color.Black)) {
            when (cameraState) {
                CameraState.CHECKING -> Box(
                    Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        "Preparing camera…",
                        color = Color.White,
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }

                CameraState.DENIED -> StatusScreen(
                    icon = Icons.Rounded.CameraAlt,
                    title = "Camera access is off",
                    message = "Allow camera access in Settings, or type the pairing " +
                        "code instead.",
                    primary = StatusAction("Open settings") { openAppSettings(context) },
                    secondary = StatusAction("Type it instead", onDismiss),
                )

                CameraState.READY -> {
                    CameraFeed(
                        onPayload = { payload ->
                            try {
                                onScan(PairingRequest.fromUri(Uri.parse(payload)))
                            } catch (e: PairingLinkException) {
                                scanError = e.message
                            } catch (_: Exception) {
                                scanError = "This is not a Hi Agent pairing link."
                            }
                        },
                    )
                    ScannerReticle()

                    Column(
                        Modifier
                            .align(Alignment.BottomCenter)
                            .fillMaxWidth()
                            .padding(horizontal = 24.dp, vertical = 44.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        scanError?.let {
                            Text(
                                text = it,
                                color = Color.White,
                                textAlign = TextAlign.Center,
                                style = MaterialTheme.typography.bodySmall,
                                modifier = Modifier
                                    .background(
                                        Color(0xD9D32F2F),
                                        RoundedCornerShape(14.dp),
                                    )
                                    .padding(horizontal = 16.dp, vertical = 12.dp),
                            )
                        }
                        Text(
                            text = "Point the camera at the pairing code your core " +
                                "is showing.",
                            color = Color.White,
                            textAlign = TextAlign.Center,
                            style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier
                                .padding(top = 10.dp)
                                .background(Color(0x80000000), RoundedCornerShape(16.dp))
                                .padding(horizontal = 18.dp, vertical = 12.dp),
                        )
                    }
                }
            }

            androidx.compose.material3.TextButton(
                onClick = onDismiss,
                modifier = Modifier
                    .align(Alignment.TopStart)
                    .padding(8.dp)
                    .then(Modifier),
            ) {
                Text("Cancel", color = Color.White)
            }
        }
    }
}

@Composable
private fun CameraFeed(onPayload: (String) -> Unit) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val executor = remember { Executors.newSingleThreadExecutor() }
    val previewView = remember { PreviewView(context) }

    DisposableEffect(Unit) {
        onDispose { executor.shutdown() }
    }

    AndroidView(
        modifier = Modifier.fillMaxSize(),
        factory = { previewView },
        update = { view ->
            val future = ProcessCameraProvider.getInstance(context)
            future.addListener({
                val provider = future.get()
                val preview = Preview.Builder().build().also {
                    it.surfaceProvider = view.surfaceProvider
                }
                val analysis = ImageAnalysis.Builder()
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()
                analysis.setAnalyzer(executor, QrAnalyzer(onPayload))

                runCatching {
                    provider.unbindAll()
                    provider.bindToLifecycle(
                        lifecycleOwner,
                        CameraSelector.DEFAULT_BACK_CAMERA,
                        preview,
                        analysis,
                    )
                }
            }, ContextCompat.getMainExecutor(context))
        },
    )
}

/**
 * One QR decode per frame, on a single background thread with
 * `STRATEGY_KEEP_ONLY_LATEST` — a backlog of stale frames would decode a code
 * the camera is no longer pointed at.
 */
private class QrAnalyzer(private val onPayload: (String) -> Unit) : ImageAnalysis.Analyzer {
    private val reader = QRCodeReader()
    private val hints = mapOf(DecodeHintType.TRY_HARDER to true)
    private var lastPayload: String? = null

    override fun analyze(image: ImageProxy) {
        try {
            // The Y plane of YUV_420_888 is the luminance ZXing wants, so there
            // is no colour conversion to do — just the plane, at its row stride.
            val plane = image.planes.firstOrNull() ?: return
            val buffer = plane.buffer
            val bytes = ByteArray(buffer.remaining()).also { buffer.get(it) }
            val source = PlanarYUVLuminanceSource(
                bytes,
                plane.rowStride,
                image.height,
                0,
                0,
                image.width.coerceAtMost(plane.rowStride),
                image.height,
                false,
            )
            val result = reader.decode(BinaryBitmap(HybridBinarizer(source)), hints)
            val text: String? = result.text
            if (text != null && text != lastPayload) {
                lastPayload = text
                onPayload(text)
            }
        } catch (_: Exception) {
            // Not-found is the overwhelmingly common case — most frames have no
            // code in them. Nothing to report and nothing to log.
        } finally {
            reader.reset()
            image.close()
        }
    }
}

/**
 * Dims everything except the square you are meant to aim with. Without it the
 * screen is a live camera feed with a caption — nothing says where to point.
 */
@Composable
private fun ScannerReticle() {
    Canvas(Modifier.fillMaxSize()) {
        val side = 250.dp.toPx()
        val topLeft = Offset((size.width - side) / 2f, (size.height - side) / 2f)
        val corner = CornerRadius(28.dp.toPx())

        drawRect(Color.Black.copy(alpha = 0.45f))
        drawRoundRect(
            color = Color.Transparent,
            topLeft = topLeft,
            size = Size(side, side),
            cornerRadius = corner,
            blendMode = BlendMode.Clear,
        )
        drawRoundRect(
            color = Color.White.copy(alpha = 0.85f),
            topLeft = topLeft,
            size = Size(side, side),
            cornerRadius = corner,
            style = Stroke(width = 2.dp.toPx()),
        )
    }
}

private fun hasCameraPermission(context: Context): Boolean =
    ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
        PackageManager.PERMISSION_GRANTED

private fun openAppSettings(context: Context) {
    runCatching {
        context.startActivity(
            Intent(
                Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                Uri.fromParts("package", context.packageName, null),
            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }
}
