package com.quietsoftware.relay.ui.pair

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.lifecycle.awaitInstance
import androidx.camera.view.PreviewView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import com.quietsoftware.relay.ui.kit.Key
import com.quietsoftware.relay.ui.kit.KeyKind
import com.quietsoftware.relay.ui.kit.T
import com.quietsoftware.relay.ui.theme.Relay
import java.util.concurrent.Executors

/**
 * The camera, looking for the `relay://pair` QR that `relay remote pair` prints. Frames are
 * decoded on the phone by ZXing; nothing leaves it. [onCode] gets each distinct text found.
 */
@Composable
fun QrScanner(onCode: (String) -> Unit, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    var granted by remember { mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }
    LaunchedEffect(Unit) { if (!granted) ask.launch(Manifest.permission.CAMERA) }
    if (!granted) {
        Box(modifier.background(Relay.colors.screen), contentAlignment = Alignment.Center) {
            Key("Allow the camera", { ask.launch(Manifest.permission.CAMERA) }, kind = KeyKind.Plain, glyph = "qr")
        }
        return
    }
    val owner = LocalLifecycleOwner.current
    val latest by rememberUpdatedState(onCode)
    val executor = remember { Executors.newSingleThreadExecutor() }
    val provider by produceState<ProcessCameraProvider?>(null) { value = runCatching { ProcessCameraProvider.awaitInstance(context) }.getOrNull() }
    val view = remember { PreviewView(context).apply { scaleType = PreviewView.ScaleType.FILL_CENTER } }
    DisposableEffect(provider) {
        val p = provider
        if (p != null) {
            val preview = Preview.Builder().build().also { it.surfaceProvider = view.surfaceProvider }
            val reader = QRCodeReader()
            val hints = mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE), DecodeHintType.TRY_HARDER to true)
            var last: String? = null
            val analysis = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
            analysis.setAnalyzer(executor) { image ->
                val text = decode(image, reader, hints)
                image.close()
                if (text != null && text != last) {
                    last = text
                    view.post { latest(text) }
                }
            }
            runCatching {
                p.unbindAll()
                p.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
            }
        }
        onDispose {
            p?.unbindAll()
        }
    }
    DisposableEffect(Unit) { onDispose { executor.shutdown() } }
    Box(modifier.background(Relay.colors.screen)) {
        AndroidView(factory = { view }, modifier = Modifier.fillMaxSize())
        T("Point at the code relay remote pair printed", Modifier.align(Alignment.BottomCenter).padding(bottom = 10.dp), Relay.type.caption, Relay.colors.ink2)
    }
}

/** The luma plane is all a QR reader needs; rows are copied when the plane is padded. */
private fun decode(image: ImageProxy, reader: QRCodeReader, hints: Map<DecodeHintType, Any>): String? {
    val plane = image.planes.firstOrNull() ?: return null
    val w = image.width
    val h = image.height
    val buffer = plane.buffer
    val stride = plane.rowStride
    val data = ByteArray(w * h)
    if (stride == w) {
        buffer.get(data, 0, minOf(data.size, buffer.remaining()))
    } else {
        for (row in 0 until h) {
            buffer.position(row * stride)
            buffer.get(data, row * w, w)
        }
    }
    val source = PlanarYUVLuminanceSource(data, w, h, 0, 0, w, h, false)
    // A terminal draws the code light on dark (pairlink.rs), the inverse of a printed one.
    for (candidate in listOf(source, source.invert())) {
        try {
            return reader.decode(BinaryBitmap(HybridBinarizer(candidate)), hints).text
        } catch (_: Exception) {
            // Not in this frame, or not this way round.
        } finally {
            reader.reset()
        }
    }
    return null
}
