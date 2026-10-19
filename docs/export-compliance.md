# Criptografia e controlo de exportações / Encryption and export controls

Documento de apoio para responder a perguntas sobre a criptografia da SEALER (app QT):
lojas de apps, distribuidores ou autoridades. **Não é aconselhamento jurídico.** Para uma
submissão formal (por exemplo, uma classificação junto do BIS dos EUA), validar com um
especialista em controlo de exportações.

Supporting document for questions about the encryption in SEALER (QT app): app stores,
distributors or authorities. **This is not legal advice.** For a formal filing (for example,
a classification request to the US BIS), have it reviewed by an export-control specialist.

---

## Português

### O produto

A SEALER é uma app de mensagens pessoais (Android; iPhone em preparação) com cifra ponta a
ponta. Serve só para comunicação entre pessoas: texto, imagens e GIFs. Não é uma ferramenta
de segurança de redes, não cifra outros dados do telemóvel e não oferece VPN nem cifra de
ficheiros arbitrários.

- Código-fonte público e gratuito: <https://github.com/moral666/QT>
- A app é gratuita e destinada ao público em geral (mercado de massas).
- Sem números de telefone nem emails: cada pessoa é identificada por um par de chaves criado
  no próprio telemóvel.

### Algoritmos usados

Todos são normas públicas e publicadas. Nenhum é criptografia proprietária ou não normalizada.

| Uso | Algoritmo | Norma |
|---|---|---|
| Acordo de chaves inicial (clássico) | X3DH com X25519 | Signal X3DH; RFC 7748 |
| Acordo de chaves inicial (pós-quântico, híbrido) | PQXDH: X3DH mais ML-KEM-768 | Signal PQXDH; NIST FIPS 203 |
| Cifra das mensagens, com chaves novas a cada mensagem | Double Ratchet | Signal Double Ratchet |
| Cifra autenticada | ChaCha20-Poly1305 | RFC 8439 |
| Derivação de chaves | HKDF-SHA256 | RFC 5869; FIPS 180-4 |
| Assinaturas (identidade, autenticação no servidor) | Ed25519 | RFC 8032 |
| Canal até ao servidor | Noise `XX_25519_ChaChaPoly_SHA256` sobre WebSocket com TLS | Noise Protocol Framework; TLS 1.2 e 1.3 |
| Base de dados no telemóvel | SQLCipher (AES-256) | FIPS 197 |
| Proteção da chave da base de dados | Android Keystore, AES-256-GCM | FIPS 197; NIST SP 800-38D |
| Código de bloqueio da app | Argon2id | RFC 9106 |

O servidor (relay) não consegue ler as mensagens nem saber quem as enviou (*sealed sender*):
só encaminha envelopes cifrados.

### Enquadramento nos EUA (EAR)

A Google Play aloja as apps em servidores nos EUA, por isso um download a partir de fora dos
EUA conta como exportação dos EUA. A Play Console não tem questionário de criptografia; a
Google diz que cada programador é responsável pelo seu próprio cumprimento.

1. **Código-fonte publicamente disponível.** O código está publicado sem restrições
   (EAR §734.3(b)(3) e §734.7). Desde a regra do BIS de 29 de março de 2021, o código-fonte de
   criptografia publicamente disponível deixou de precisar da notificação por email ao BIS e à
   NSA (antes exigida em §742.15(b)), **exceto se usar "criptografia não normalizada"**. A
   SEALER só usa algoritmos normalizados (tabela acima), por isso não é precisa notificação.
2. **App compilada distribuída na loja.** É compilada a partir desse código público e
   distribuída de graça a qualquer pessoa. Se não for considerada publicamente disponível, o
   enquadramento seguinte é o de **mercado de massas**: ECCN 5D992.c, ao abrigo de
   §740.17(b)(1), por autoclassificação. Com a mesma regra de 2021, este caso deixou de exigir
   o relatório anual de autoclassificação (mais uma vez, salvo criptografia não normalizada).
3. Não é aplicável a "Note 4" (itens cuja função principal não é a segurança da informação):
   a cifra das mensagens é uma função central da app. O enquadramento correto é o de cima, e
   não o de "comunicações interpessoais".
4. A Google bloqueia os downloads para os países sob embargo dos EUA.

### Enquadramento na União Europeia

O programador está em Portugal, onde se aplica o Regulamento (UE) 2021/821 (bens de dupla
utilização). No Anexo I, Categoria 5, Parte 2, a app fica fora de controlo por duas vias:
a **Nota de Criptografia** (Nota 3: produtos de mercado de massas, vendidos ao público sem
restrições) e a **Nota Geral sobre Software** (software do domínio público).

### App Store (iPhone)

O `Info.plist` declara `ITSAppUsesNonExemptEncryption = YES`. Na App Store Connect, as respostas
são as de baixo: criptografia normalizada (a mesma tabela; no iPhone a chave da base de dados é
protegida pelo Secure Enclave com ECIES P-256 e AES-GCM), código aberto e público, mercado de massas.
A Apple pode pedir a autoclassificação (ECCN 5D992.c) ou a declaração de França (ANSSI) para a
distribuição em França.

### Respostas curtas

- Usa criptografia? **Sim**, para proteger mensagens pessoais.
- Criptografia proprietária ou não normalizada? **Não.**
- Código aberto e público? **Sim.**
- Disponível ao público em geral, de graça? **Sim.**

---

## English

### The product

SEALER is a personal messaging app (Android; iPhone in preparation) with end-to-end
encryption. It is only for communication between people: text, images and GIFs. It is not a
network security tool, does not encrypt other data on the phone and offers no VPN or
general-purpose file encryption.

- Free, public source code: <https://github.com/moral666/QT>
- The app is free and intended for the general public (mass market).
- No phone numbers or email addresses: each person is identified by a key pair generated on
  their own phone.

### Algorithms

All are public, published standards. None is proprietary or non-standard cryptography.

| Use | Algorithm | Standard |
|---|---|---|
| Initial key agreement (classical) | X3DH with X25519 | Signal X3DH; RFC 7748 |
| Initial key agreement (post-quantum, hybrid) | PQXDH: X3DH plus ML-KEM-768 | Signal PQXDH; NIST FIPS 203 |
| Message encryption, with new keys for every message | Double Ratchet | Signal Double Ratchet |
| Authenticated encryption | ChaCha20-Poly1305 | RFC 8439 |
| Key derivation | HKDF-SHA256 | RFC 5869; FIPS 180-4 |
| Signatures (identity, server authentication) | Ed25519 | RFC 8032 |
| Channel to the server | Noise `XX_25519_ChaChaPoly_SHA256` over WebSocket with TLS | Noise Protocol Framework; TLS 1.2 and 1.3 |
| On-device database | SQLCipher (AES-256) | FIPS 197 |
| Database key protection | Android Keystore, AES-256-GCM | FIPS 197; NIST SP 800-38D |
| App lock code | Argon2id | RFC 9106 |

The server (relay) cannot read messages or learn who sent them (sealed sender): it only
forwards encrypted envelopes.

### United States (EAR)

Google Play hosts apps on servers in the US, so a download from outside the US counts as a US
export. The Play Console has no encryption questionnaire; Google states that each developer is
responsible for their own compliance.

1. **Publicly available source code.** The source code is published without restrictions
   (EAR §734.3(b)(3) and §734.7). Since the BIS rule of 29 March 2021, publicly available
   encryption source code no longer requires the email notification to BIS and NSA (previously
   required under §742.15(b)), **except when it implements "non-standard cryptography"**.
   SEALER uses only standard algorithms (table above), so no notification is required.
2. **Compiled app distributed through the store.** It is built from that public source code
   and distributed free of charge to anyone. If it is not treated as publicly available, the
   next classification is **mass market**: ECCN 5D992.c under §740.17(b)(1), by
   self-classification. Under the same 2021 rule, this no longer requires the annual
   self-classification report (again, unless non-standard cryptography is used).
3. "Note 4" (items whose primary function is not information security) does not apply:
   message encryption is a core function of the app. The correct basis is the one above, not
   an "interpersonal communications" exclusion.
4. Google blocks downloads to US-embargoed countries.

### European Union

The developer is based in Portugal, where Regulation (EU) 2021/821 (dual-use items) applies.
Under Annex I, Category 5, Part 2, the app is outside control through two routes: the
**Cryptography Note** (Note 3: mass-market products sold to the public without restriction)
and the **General Software Note** (software in the public domain).

### Short answers

- Does it use encryption? **Yes**, to protect personal messages.
- Proprietary or non-standard cryptography? **No.**
- Open and public source code? **Yes.**
- Available to the general public, free of charge? **Yes.**

---

Fontes / Sources:
[Google Play: export compliance](https://support.google.com/googleplay/android-developer/answer/113770) ·
[BIS 2021 encryption rule (Thompson Hine)](https://www.thompsonhine.com/publications/bis-eliminates-most-mass-market-encryption-reporting-obligations)
