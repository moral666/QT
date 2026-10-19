#!/usr/bin/env python3
"""Textos da app iPhone a partir dos da app Android (as mesmas chaves, nas seis linguas).

    python3 scripts/strings-ios.py

Le android/app/src/main/res/values*/strings.xml e escreve
ios/SealerMessenger/Resources/<lingua>.lproj/Localizable.strings e InfoPlist.strings.
Correr sempre que mudar um texto no Android (ou os textos so do iPhone, em EXTRA, abaixo).
Os ficheiros gerados vao para o git (o Mac nao precisa de Python para compilar).
"""
import os
import re
import sys
import xml.etree.ElementTree as ET

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RES = os.path.join(ROOT, "android", "app", "src", "main", "res")
OUT = os.path.join(ROOT, "ios", "SealerMessenger", "Resources")

# Lingua do Android -> pasta .lproj do iPhone (PT e a base, como no Android).
LANGS = {"": "pt-PT", "-en": "en", "-fr": "fr", "-es": "es", "-de": "de", "-it": "it"}

# Textos so do iPhone (nao existem no Android). Ordem: pt, en, fr, es, de, it.
EXTRA = {
    # Notificacao das mensagens: a Apple so recebe esta chave, o texto e traduzido no iPhone.
    "push_new": ("Tens mensagens novas", "You have new messages", "Vous avez de nouveaux messages",
                 "Tienes mensajes nuevos", "Du hast neue Nachrichten", "Hai nuovi messaggi"),
    "ios_open_settings": ("Abrir Definições", "Open Settings", "Ouvrir Réglages", "Abrir Ajustes",
                          "Einstellungen öffnen", "Apri Impostazioni"),
    "ios_paste_image": ("Colar imagem ou GIF", "Paste image or GIF", "Coller une image ou un GIF",
                        "Pegar imagen o GIF", "Bild oder GIF einfügen", "Incolla immagine o GIF"),
    "ios_no_image_to_paste": ("Não há nenhuma imagem copiada.", "There is no copied image.",
                              "Aucune image copiée.", "No hay ninguna imagen copiada.",
                              "Es wurde kein Bild kopiert.", "Nessuna immagine copiata."),
    "ios_no_image_to_paste_hint": (
        "No teclado de GIFs, toca no GIF para o copiar e depois escolhe Colar imagem ou GIF.",
        "In the GIF keyboard, tap a GIF to copy it, then choose Paste image or GIF.",
        "Dans le clavier de GIF, touchez un GIF pour le copier, puis choisissez Coller une image ou un GIF.",
        "En el teclado de GIF, toca un GIF para copiarlo y luego elige Pegar imagen o GIF.",
        "Tippe in der GIF-Tastatur auf ein GIF, um es zu kopieren, und wähle dann Bild oder GIF einfügen.",
        "Nella tastiera GIF, tocca una GIF per copiarla e poi scegli Incolla immagine o GIF."),
    "ios_record_voice": ("Gravar mensagem de voz", "Record voice message", "Enregistrer un message vocal",
                         "Grabar mensaje de voz", "Sprachnachricht aufnehmen", "Registra messaggio vocale"),
    "ios_quick_unlock": ("Face ID, Touch ID ou código do iPhone", "Face ID, Touch ID or iPhone passcode",
                         "Face ID, Touch ID ou code de l'iPhone", "Face ID, Touch ID o código del iPhone",
                         "Face ID, Touch ID oder iPhone-Code", "Face ID, Touch ID o codice dell'iPhone"),
    "ios_quick_unlock_hint": ("Para desbloquear sem escrever o código da app.", "To unlock without typing the app code.",
                              "Pour déverrouiller sans saisir le code de l'app.", "Para desbloquear sin escribir el código de la app.",
                              "Zum Entsperren ohne den App-Code einzugeben.", "Per sbloccare senza digitare il codice dell'app."),
    "ios_bio_unavailable": ("Indisponível: este iPhone não tem código de bloqueio.",
                            "Unavailable: this iPhone has no passcode.",
                            "Indisponible : cet iPhone n'a pas de code.",
                            "No disponible: este iPhone no tiene código.",
                            "Nicht verfügbar: Dieses iPhone hat keinen Code.",
                            "Non disponibile: questo iPhone non ha un codice."),
    "ios_mic_denied_call": (
        "Sem acesso ao microfone não é possível fazer chamadas. Podes ativá-lo nas Definições do iPhone.",
        "Without microphone access you cannot make calls. You can turn it on in the iPhone Settings.",
        "Sans accès au micro, impossible de passer des appels. Vous pouvez l'activer dans les Réglages de l'iPhone.",
        "Sin acceso al micrófono no se pueden hacer llamadas. Puedes activarlo en los Ajustes del iPhone.",
        "Ohne Mikrofonzugriff sind keine Anrufe möglich. Du kannst ihn in den iPhone-Einstellungen aktivieren.",
        "Senza accesso al microfono non è possibile chiamare. Puoi attivarlo nelle Impostazioni dell'iPhone."),
    "ios_bio_confirm": (
        "Confirma com o teu código. A seguir, o iPhone pede o Face ID, o Touch ID ou o código do iPhone.",
        "Confirm with your code. Next, the iPhone asks for Face ID, Touch ID or the iPhone passcode.",
        "Confirmez avec votre code. Ensuite, l'iPhone demande Face ID, Touch ID ou le code de l'iPhone.",
        "Confirma con tu código. Después, el iPhone pide Face ID, Touch ID o el código del iPhone.",
        "Bestätige mit deinem Code. Danach fragt das iPhone nach Face ID, Touch ID oder dem iPhone-Code.",
        "Conferma con il tuo codice. Poi l'iPhone chiede Face ID, Touch ID o il codice dell'iPhone."),
    "ios_bio_changed": (
        "O Face ID, o Touch ID ou o código do iPhone mudaram. Usa o código da app e volta a ativar o desbloqueio rápido.",
        "Face ID, Touch ID or the iPhone passcode changed. Use the app code and turn quick unlock on again.",
        "Face ID, Touch ID ou le code de l'iPhone ont changé. Utilisez le code de l'app et réactivez le déverrouillage rapide.",
        "Face ID, Touch ID o el código del iPhone han cambiado. Usa el código de la app y vuelve a activar el desbloqueo rápido.",
        "Face ID, Touch ID oder der iPhone-Code haben sich geändert. Nutze den App-Code und aktiviere die Schnellentsperrung erneut.",
        "Face ID, Touch ID o il codice dell'iPhone sono cambiati. Usa il codice dell'app e riattiva lo sblocco rapido."),
    "ios_use_face_id": ("Usar Face ID", "Use Face ID", "Utiliser Face ID", "Usar Face ID", "Face ID verwenden", "Usa Face ID"),
    "ios_use_touch_id": ("Usar Touch ID", "Use Touch ID", "Utiliser Touch ID", "Usar Touch ID", "Touch ID verwenden", "Usa Touch ID"),
    "ios_use_passcode": ("Usar o código do iPhone", "Use the iPhone passcode", "Utiliser le code de l'iPhone",
                         "Usar el código del iPhone", "iPhone-Code verwenden", "Usa il codice dell'iPhone"),
}

# Textos dos pedidos de permissao do iOS (Info.plist).
PLIST = {
    "NSCameraUsageDescription": (
        "Para ler o código QR de outra pessoa e adicioná-la aos contactos.",
        "To scan another person's QR code and add them to your contacts.",
        "Pour scanner le code QR d'une autre personne et l'ajouter à vos contacts.",
        "Para leer el código QR de otra persona y añadirla a tus contactos.",
        "Um den QR-Code einer anderen Person zu scannen und sie zu deinen Kontakten hinzuzufügen.",
        "Per leggere il codice QR di un'altra persona e aggiungerla ai contatti."),
    "NSMicrophoneUsageDescription": (
        "Para as chamadas de voz e as mensagens de voz.",
        "For voice calls and voice messages.",
        "Pour les appels vocaux et les messages vocaux.",
        "Para las llamadas de voz y los mensajes de voz.",
        "Für Sprachanrufe und Sprachnachrichten.",
        "Per le chiamate vocali e i messaggi vocali."),
    "NSFaceIDUsageDescription": (
        "Para desbloquear a SEALER sem escrever o código.",
        "To unlock SEALER without typing the code.",
        "Pour déverrouiller SEALER sans saisir le code.",
        "Para desbloquear SEALER sin escribir el código.",
        "Um SEALER ohne Code-Eingabe zu entsperren.",
        "Per sbloccare SEALER senza digitare il codice."),
}

ORDER = ["pt-PT", "en", "fr", "es", "de", "it"]


def android_text(raw: str) -> str:
    """Tira os escapes do Android (\\' \\" \\n \\@ \\?)."""
    out, i = [], 0
    while i < len(raw):
        ch = raw[i]
        if ch == "\\" and i + 1 < len(raw):
            nxt = raw[i + 1]
            out.append({"n": "\n", "t": "\t"}.get(nxt, nxt))
            i += 2
            continue
        out.append(ch)
        i += 1
    s = "".join(out)
    if len(s) >= 2 and s[0] == '"' and s[-1] == '"':
        s = s[1:-1]
    return s


def ios_format(s: str) -> str:
    # %1$s -> %1$@ (texto). %s -> %@. %d mantem-se. %% mantem-se.
    s = re.sub(r"%(\d+\$)?s", lambda m: "%" + (m.group(1) or "") + "@", s)
    return s


def strings_escape(s: str) -> str:
    return s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t")


def read_android(suffix: str) -> dict:
    path = os.path.join(RES, "values" + suffix, "strings.xml")
    tree = ET.parse(path)
    out = {}
    for el in tree.getroot().findall("string"):
        if el.get("translatable") == "false":
            continue
        text = "".join(el.itertext())
        out[el.get("name")] = ios_format(android_text(text))
    return out


def main() -> int:
    base = read_android("")
    for suffix, lproj in LANGS.items():
        values = dict(base)               # o que faltar numa lingua fica em portugues
        values.update(read_android(suffix) if suffix else {})
        idx = ORDER.index(lproj)
        for k, v in EXTRA.items():
            values[k] = v[idx]
        d = os.path.join(OUT, lproj + ".lproj")
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "Localizable.strings"), "w", encoding="utf-8") as f:
            f.write("/* Gerado por scripts/strings-ios.py a partir do Android. Nao editar a mao. */\n")
            for k in sorted(values):
                f.write(f'"{k}" = "{strings_escape(values[k])}";\n')
        with open(os.path.join(d, "InfoPlist.strings"), "w", encoding="utf-8") as f:
            f.write("/* Gerado por scripts/strings-ios.py. Nao editar a mao. */\n")
            for k, v in PLIST.items():
                f.write(f'"{k}" = "{strings_escape(v[idx])}";\n')
        print(f"{lproj}: {len(values)} textos")
    return 0


if __name__ == "__main__":
    sys.exit(main())
