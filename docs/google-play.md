# Google Play: o que preencher na Play Console

Respostas para a app SEALER **sem notificações FCM** (versão atual, 0.8.0). Se um dia a
app passar a usar FCM, ver a secção no fim: algumas respostas mudam.

Endereços que a Play pede:

- Política de privacidade: `https://sealer.pt/privacidade`
- Apagar a conta (link web): `https://sealer.pt/apagar-conta`

---

## 1. Segurança dos dados (Data safety)

Regra da Google em que isto se apoia: dados com cifra ponta a ponta (ilegíveis para todos
menos quem envia e quem recebe, incluindo o programador) **não contam** como recolhidos.
Dados pseudónimos **contam**. Por isso as mensagens e as imagens não se declaram, mas o ID
da pessoa (a chave pública que o servidor guarda) declara-se.

### Recolha e segurança

| Pergunta | Resposta |
|---|---|
| A app recolhe ou partilha algum dos tipos de dados exigidos? | **Sim** (só o ID, ver abaixo) |
| Todos os dados recolhidos são cifrados em trânsito? | **Sim** (TLS mais Noise até ao servidor) |
| Que métodos de criação de conta a app suporta? | **Outro**: "A conta é um par de chaves criado no telemóvel, sem nome de utilizador, palavra-passe, telefone nem email." |
| Dás aos utilizadores uma forma de pedir a eliminação dos dados? | **Sim** |
| URL para apagar a conta | `https://sealer.pt/apagar-conta` |

### Tipos de dados

Marcar **só** isto:

**Informações pessoais → IDs de utilizador (User IDs)**

| Pergunta | Resposta |
|---|---|
| Recolhido | **Sim** |
| Partilhado | **Não** |
| Processado de forma efémera | **Não** (o servidor guarda-o até 90 dias) |
| Obrigatório ou opcional | **Obrigatório** |
| Finalidade | **Funcionalidade da app** |

Porquê: o servidor guarda o ID (a chave pública de identidade) e o pacote de chaves públicas
para que outros possam iniciar uma conversa, e usa o ID para guardar as mensagens à espera.

**Não marcar** (e porquê):

- Mensagens, fotos, imagens, GIFs: cifra ponta a ponta, o servidor não os consegue ler.
- Endereço IP: só é usado em memória para manter a ligação; não fica registado.
- Contactos: ficam só no telemóvel, cifrados.
- Localização, identificadores do dispositivo, atividade na app, diagnósticos, falhas: a app
  não recolhe nada disto. Não há estatísticas, publicidade nem relatórios de falhas.

---

## 2. Eliminação da conta (Account deletion)

- **Na app:** menu → **Destruir perfil e dados**. Apaga primeiro no servidor (chaves
  públicas e mensagens em espera) e depois tudo do telemóvel.
- **Na web:** `https://sealer.pt/apagar-conta`. Como não há email nem palavra-passe, a
  página explica como apagar na app e o que acontece se a app já foi desinstalada (o
  servidor apaga tudo sozinho: mensagens ao fim de 30 dias, chaves públicas ao fim de 90).
- **O que fica depois de apagar:** durante 30 dias, o servidor lembra-se só de que aquele
  ID foi apagado, para avisar quem lhe tentar escrever. Está descrito na política de
  privacidade.

---

## 3. Conteúdo gerado por utilizadores (UGC)

A política da Play aplica-se a conversas 1 a 1. O que exige e onde está:

- **Bloquear utilizadores:** na conversa, menu ⋮ → Bloquear (também para quem não está nos
  contactos). Lista e desbloqueio em menu → Contactos bloqueados.
- **Aceitar os termos antes de usar:** ecrã "Antes de começar", com os termos
  (`https://sealer.pt/termos`) e a política de privacidade.
- **Denunciar:** os termos indicam `privacidade@sealer.pt`. As mensagens são cifradas ponta a
  ponta, por isso não há moderação de conteúdo no servidor.

## 4. Criptografia / exportação

A Play Console não tem questionário. O enquadramento está em
[`export-compliance.md`](export-compliance.md).

---

## 5. Outras secções da Play Console

| Secção | Resposta |
|---|---|
| Anúncios | **Não** tem anúncios |
| Acesso à app | Todas as funcionalidades sem restrições (não há login). Para a revisão, explicar que basta criar um ID e adicionar outro telemóvel com o QR ou o ID. |
| Aplicações governamentais | Não |
| Funcionalidades financeiras | Nenhuma |
| Saúde | Nenhuma |

## 6. Antes de publicar

- **Conta de programador:** 25 dólares, uma vez.
- **Teste fechado obrigatório** (contas pessoais novas): pelo menos **12 testers durante 14
  dias seguidos** antes de poder publicar para todos.

---

## Se a app passar a usar FCM (notificações com a app fechada)

Mudam estas respostas:

- Juntar **Dispositivo ou outros IDs (Device or other IDs)**: recolhido, não partilhado,
  obrigatório, finalidade **Funcionalidade da app** (o token do Firebase, que o servidor
  guarda para pedir à Google que acorde o telemóvel).
- Atualizar a política de privacidade: a Google passa a saber quando o telemóvel recebe um
  aviso (sem conteúdo, sem remetente).
