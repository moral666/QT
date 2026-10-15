# Marca SEALER

| Ficheiro | Para quê |
|---|---|
| `sealer-logo-transparente.png` | Logotipo com fundo transparente (documentos, site, apresentações). |
| `sealer-icone-1024.png` | Ícone quadrado 1024×1024 com fundo branco (App Store; o iPhone arredonda os cantos). |
| `play-store-icone-512.png` | Ícone 512×512 pedido pela Play Store (a Google arredonda os cantos). |

Os ícones das apps são gerados a partir do logotipo:

- Android: `android/app/src/main/res/drawable-*dpi/ic_launcher_foreground.png` (ícone adaptativo,
  com o logo dentro da zona segura de 66 dp) e `ic_launcher_monochrome.png` (ícones com o tema do
  telemóvel, Android 13+); `drawable-nodpi/sealer_logo.png` dentro da app.
- iPhone: `ios/SealerMessenger/Assets.xcassets/AppIcon.appiconset/AppIcon-1024.png` e
  `Logo.imageset/Logo.png` dentro da app.

## Fonte

O nome na barra de topo usa a **Manrope Bold** (`fontes/Manrope-Bold.ttf`), gratuita e com a
licença SIL Open Font License 1.1 (`fontes/Manrope-OFL.txt`). A licença vai dentro das apps
(`android/app/src/main/assets/licencas/` e `ios/SealerMessenger/Resources/`), como a OFL pede.
