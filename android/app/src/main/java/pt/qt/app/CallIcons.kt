package pt.qt.app

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

/**
 * Icones do ecra da chamada (0.9.38), desenhados aqui a partir dos caminhos SVG dos Material
 * Icons (Apache 2.0), como o da videochamada: a app so tem os icones base do Material e nao
 * leva bibliotecas novas por causa do tamanho.
 */
private fun callIcon(name: String, vararg paths: String): ImageVector =
    ImageVector.Builder(name = name, defaultWidth = 24.dp, defaultHeight = 24.dp, viewportWidth = 24f, viewportHeight = 24f)
        .apply { paths.forEach { addPath(pathData = addPathNodes(it), fill = SolidColor(Color.Black)) } }
        .build()

/** Microfone ligado ("Mic"). */
val MicIcon: ImageVector by lazy {
    callIcon(
        "Filled.Mic",
        "M12,14c1.66,0 2.99,-1.34 2.99,-3L15,5c0,-1.66 -1.34,-3 -3,-3S9,3.34 9,5v6c0,1.66 1.34,3 3,3z" +
            "M17.3,11c0,3 -2.54,5.1 -5.3,5.1S6.7,14 6.7,11L5,11c0,3.41 2.72,6.23 6,6.72L11,21h2v-3.28c3.28,-0.48 6,-3.3 6,-6.72h-1.7z",
    )
}

/** Microfone silenciado ("MicOff"). */
val MicOffIcon: ImageVector by lazy {
    callIcon(
        "Filled.MicOff",
        "M19,11h-1.7c0,0.74 -0.16,1.43 -0.43,2.05l1.23,1.23c0.56,-0.98 0.9,-2.09 0.9,-3.28z" +
            "M14.98,11.17c0,-0.06 0.02,-0.11 0.02,-0.17L15,5c0,-1.66 -1.34,-3 -3,-3S9,3.34 9,5v0.18l5.98,5.99z" +
            "M4.27,3L3,4.27l6.01,6.01L9,11c0,1.66 1.33,3 2.99,3 0.22,0 0.44,-0.03 0.65,-0.08l1.66,1.66" +
            "c-0.71,0.33 -1.5,0.52 -2.31,0.52 -2.76,0 -5.3,-2.1 -5.3,-5.1L5,11c0,3.41 2.72,6.23 6,6.72L11,21h2v-3.28" +
            "c0.91,-0.13 1.77,-0.45 2.54,-0.9L19.73,21 21,19.73 4.27,3z",
    )
}

/** Altifalante ("VolumeUp"). */
val SpeakerIcon: ImageVector by lazy {
    callIcon(
        "Filled.VolumeUp",
        "M3,9v6h4l5,5L12,4L7,9L3,9z" +
            "M16.5,12c0,-1.77 -1.02,-3.29 -2.5,-4.03v8.05c1.48,-0.73 2.5,-2.25 2.5,-4.02z" +
            "M14,3.23v2.06c2.89,0.86 5,3.54 5,6.71s-2.11,5.85 -5,6.71v2.06c4.01,-0.91 7,-4.49 7,-8.77s-2.99,-7.86 -7,-8.77z",
    )
}

/** Camara desligada ("VideocamOff"). A ligada e o [VideoCallIcon]. */
val VideoOffIcon: ImageVector by lazy {
    callIcon(
        "Filled.VideocamOff",
        "M21,6.5l-4,4V7c0,-0.55 -0.45,-1 -1,-1H9.82L21,17.18V6.5z" +
            "M3.27,2L2,3.27 4.73,6H4c-0.55,0 -1,0.45 -1,1v10c0,0.55 0.45,1 1,1h12c0.21,0 0.39,-0.08 0.54,-0.18L19.73,21 21,19.73 3.27,2z",
    )
}

/** Trocar a camara (frontal/traseira), "Cameraswitch". */
val SwitchCameraIcon: ImageVector by lazy {
    callIcon(
        "Filled.Cameraswitch",
        "M16,7h-1l-1,-1h-4L9,7H8C6.9,7 6,7.9 6,9v6c0,1.1 0.9,2 2,2h8c1.1,0 2,-0.9 2,-2V9C18,7.9 17.1,7 16,7z" +
            "M12,14c-1.1,0 -2,-0.9 -2,-2c0,-1.1 0.9,-2 2,-2s2,0.9 2,2C14,13.1 13.1,14 12,14z",
        "M8.57,0.51l4.48,4.48V2.04c4.72,0.47 8.48,4.23 8.95,8.95h2C23.34,3.02 15.49,-1.59 8.57,0.51z",
        "M10.95,21.96C6.23,21.49 2.47,17.73 2,13.01H0c0.66,7.97 8.51,12.58 15.43,10.48l-4.48,-4.48V21.96z",
    )
}
