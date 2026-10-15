# Testes de campo

Registo dos testes feitos com as apps reais, fora do ambiente de testes
automáticos. Cada entrada diz o que foi testado, em que condições, o que
correu bem, o que falhou e o que ficou por testar.

Um teste de campo bem sucedido mostra que o sistema **funciona e é fiável no
uso real**. Não mostra que é **seguro contra um atacante determinado**: isso
continua a exigir uma auditoria externa (ver `known-limitations.md`).

---

## 15 de outubro de 2026 — cinco aparelhos, três plataformas

**Resultado:** troca contínua de mensagens entre todos os aparelhos, incluindo
envios simultâneos e em rajada a partir de vários aparelhos ao mesmo tempo.
**Nenhuma mensagem se perdeu** durante o teste.

### Montagem

| Aparelho | Plataforma | App |
| --- | --- | --- |
| 2 telemóveis Android reais | Android | SealerMessenger `0.1-alpha`, release assinada (V2/V3) |
| 1 emulador Android (Android Studio, Pixel 9 Pro XL) | Android | a mesma APK de teste |
| 1 iPhone | iOS | SealerMessenger (SwiftUI), compilada no Xcode |
| 1 Mac | macOS, com o simulador de iPhone do Xcode | SealerMessenger (SwiftUI) |

- **Relay:** AWS EC2 `t3.small` (região eu-north-1, Estocolmo), relay atrás do
  Caddy com TLS (`wss://`, certificado Let's Encrypt), pinning da chave Noise
  nas apps, Redis só em `127.0.0.1` com ACL.
- **Núcleo:** o mesmo código Rust nas duas plataformas (PQXDH com
  X25519 + ML-KEM-768, Double Ratchet, Sealed Sender, histórico e fila de saída
  em SQLCipher). Android compilado do commit `0b2355b`; iOS do commit
  `d3a48ef` (o mesmo protocolo, ainda sem as opções de apagar).
- **Redes:** telemóveis na rede de casa e, numa fase anterior do mesmo dia,
  numa rede móvel diferente.

### O que foi exercitado

- Conversas um-para-um entre aparelhos Android, entre Android e iOS e entre
  aparelhos iOS.
- **Envios simultâneos e contínuos**, nos dois sentidos e em vários aparelhos ao
  mesmo tempo. Isto testa o Double Ratchet com mensagens a cruzarem-se, a
  entrega fora de ordem (chaves de mensagens saltadas), a fila de saída e a
  proteção contra duplicados.
- Estados das mensagens: *por enviar* → *enviada* → *entregue* → *vista*,
  através de recibos cifrados.
- Funcionalidades novas desta versão, nas apps Android: **Eliminar conversa**
  e **Apagar contacto**.

### Problemas encontrados e o que foi feito

| Problema | Causa | Estado |
| --- | --- | --- |
| Duas pessoas que se adicionavam e escreviam ao mesmo tempo não liam as mensagens uma da outra | Sessões PQXDH cruzadas: cada lado iniciou a sua | **Corrigido** antes deste teste (desempate determinístico + reenvio), com testes automáticos |
| Depois de **Apagar contacto**, as mensagens do outro lado deixavam de chegar se fosse ele a escrever primeiro | Quem apagou já não tem as chaves da conversa e as mensagens eram descartadas sem aviso | **Contornado** no teste (quem apagou volta a adicionar e escreve primeiro). Correção automática **em curso**: feita e testada no núcleo, falta ligá-la às apps |
| Mensagens não chegavam a um telemóvel noutra rede | Não confirmada. O relay e o caminho TLS estavam bons (o pedido WebSocket respondeu `101`) e o aparelho remoto chegava ao relay; causa provável: IDs de contacto desatualizados | Deixou de acontecer sem alteração de código |
| App iOS ficou em branco no primeiro arranque no simulador | Não confirmada (primeiro arranque do simulador, Mac sob carga) | Não voltou a acontecer; a app abriu e trocou mensagens |
| A compilação Android falhou num PC com dois NDK instalados | O NDK r26d é antigo demais para o OpenSSL incluído | Usar o NDK 27 ou mais recente (`ANDROID_NDK_HOME`) |

### Não registado neste teste

- Modelos exatos e versões do sistema dos telemóveis e do iPhone.
- Duração do teste e número total de mensagens.
- Latência medida (para números de latência e débito ver
  `server/examples/loadtest.rs`, que mede o mesmo caminho que as apps).

### Ainda por testar

- Notificações (as apps só recebem mensagens enquanto estão abertas).
- Muitos utilizadores em simultâneo no relay da AWS (teste de carga com
  `loadtest` e `conntest` contra a instância real). Numa máquina local, o relay
  aguentou 10 000 ligações em simultâneo: ver `performance.md`.
- Mensagens longas perto do limite (cerca de 9 000 caracteres) e a fila do
  relay cheia (1 000 mensagens à espera para a mesma pessoa).
- Redes instáveis (perda de rede a meio de um envio, modo avião).
- Atualização da app por cima de uma versão antiga, com conversas já existentes.
- Interoperabilidade com o cliente de terminal `messenger` contra o relay da AWS
  (já testada com um relay local nos testes automáticos).
