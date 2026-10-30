package pt.qt.app

/*
 * Saidas do som e fim das chamadas, como o sistema de chamadas do Android (Telecom) as da.
 * So logica, sem Android: os testes unitarios correm no PC (CallRoutesTest).
 */

/** Tipo de saida do som de uma chamada. */
enum class RouteKind { PHONE, SPEAKER, BLUETOOTH, WIRED, OTHER }

/** Uma saida do som: `id` e o do sistema, `name` o que aparece na lista (Bluetooth: o nome do aparelho). */
data class AudioRoute(val id: String, val kind: RouteKind, val name: String)

/** Ha Bluetooth ou auscultadores: o botao do altifalante abre a lista curta em vez de alternar. */
fun List<AudioRoute>.needsMenu(): Boolean = any { it.kind == RouteKind.BLUETOOTH || it.kind == RouteKind.WIRED }

/** Sem Bluetooth nem auscultadores: altifalante <-> telemovel. `null`: nao ha para onde mudar. */
fun List<AudioRoute>.toggleTarget(current: AudioRoute?): AudioRoute? =
    if (current?.kind == RouteKind.SPEAKER) {
        firstOrNull { it.kind == RouteKind.PHONE }
    } else {
        firstOrNull { it.kind == RouteKind.SPEAKER }
    }

/** Ordem da lista: telemovel (ou auscultadores), altifalante, aparelhos Bluetooth. */
fun List<AudioRoute>.menuOrder(): List<AudioRoute> = sortedBy {
    when (it.kind) {
        RouteKind.PHONE -> 0
        RouteKind.WIRED -> 1
        RouteKind.SPEAKER -> 2
        RouteKind.BLUETOOTH -> 3
        RouteKind.OTHER -> 4
    }
}

/** Como a chamada acabou, para o sistema (passa a DisconnectCause em SystemCalls). */
enum class CallEnd { LOCAL, REMOTE, REJECTED, BUSY, MISSED, CANCELED, ERROR }
