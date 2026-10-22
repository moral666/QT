# Sealer Control

Junta numa só app as três ferramentas antigas (`sealer_aws_monitor`, `Site Sealer status` e `BADGEv2`).

- **Relay AWS:** estado da EC2, CPU, RAM, disco, ligações, serviço do relay, erros, custo do mês e alertas. Mostra também a latência real lida do `estado.json`: o envio, a entrega e a soma entre telemóveis. Se a entrega for de há mais de 1 hora, aparece marcada como antiga.
- **Site:** verifica o sealer.pt a cada 60 s e mostra a disponibilidade, o tempo de resposta e o histórico de mudanças.
- **Badges:** valida o `qt1:…` e emite o badge sem passar pelo shell, com confirmação antes de retirar e um registo do resultado.
- **Denúncias:** lista as denúncias das apps (abertas neste PC com a chave do operador, pela ferramenta `cargo run --example denuncias`), apaga-as e bane ou desbane contas no relay, sempre com confirmação. Caminho da chave em Definições → Denúncias.
- **Premium:** dá premium a uma conta (cola o `qt1:…`, escolhe 1 mês, 3 meses, 1 ano ou sem fim) ou tira-o, e mostra quem tem. Usa a mesma chave do operador.
- **Bandeja e notificações:** recebes um aviso quando o site cai, o relay para ou a AWS dá alerta, e outro quando recupera. Fechar a janela deixa a app a correr na bandeja.

## Correr a partir do código

```bash
cd ~/QT-repo/tools/sealer-control
python3 -m pip install -r requirements.txt
python3 sealer-control.py
```

## Gerar o executável

```bash
./build.sh            # cria dist/sealer-control
cp dist/sealer-control ~/.local/bin/
```

## Credenciais AWS

A app usa o perfil do AWS CLI, por isso nunca escrevas chaves na app. Configura o perfil com:

```bash
aws configure --profile sealer-monitor     # região: eu-north-1
```

Depois, em Definições → Perfil AWS CLI, escreve `sealer-monitor`. As permissões mínimas estão em `iam-policy.json`.

## Chamadas à AWS (com atualização a cada 60 s)

| O quê | Frequência | Custo |
|---|---|---|
| Status checks EC2 | cada atualização | grátis |
| Métricas do agente (5) | cada atualização | ~216 mil por mês, dentro do 1 milhão grátis do CloudWatch |
| Métricas EC2 (3) | no máximo a cada 5 min | ~26 mil por mês |
| Lista de instâncias | no máximo a cada 10 min | grátis |
| Cost Explorer | no máximo a cada 6 h, com cache em disco | 0,01 USD por pedido, ~1,20 USD por mês |

Não uso GetMetricData porque não entra no plano gratuito do CloudWatch.

## Testes

```bash
PYTHONPATH=. python3 tests/test_logic.py      # lógica (estado.json, badges, alertas)
PYTHONPATH=. python3 tests/screens.py         # abre a app com dados simulados e grava capturas
```

## Ficheiros

`~/.config/sealer-control/config.json` e `cache.json`, ambos com modo 600. Na primeira execução, as definições do `sealer-aws-monitor` antigo são importadas.
