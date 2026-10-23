# Alarme de orçamento na AWS (Budgets + CloudWatch)

Guia para o Nuno criar, na conta AWS da SEALER, um orçamento mensal com avisos por email e um alarme para o tráfego de saída da EC2 do relay. Nada disto muda o relay nem as apps.

**Importante:** um orçamento da AWS só **avisa**, não desliga nada nem impede gastos. Os números da faturação são atualizados poucas vezes por dia, por isso um aviso do orçamento pode chegar com horas de atraso. O alarme do CloudWatch (parte 2) vê o tráfego da EC2 quase em tempo real e serve para apanhar um pico mais cedo.

## O que vais criar

| O quê | Onde | Custo |
| --- | --- | --- |
| Orçamento mensal de custos (sugestão: 10 €, ajustável) com 4 avisos por email: 50 %, 80 % e 100 % do gasto real, e 100 % do gasto previsto | AWS Budgets (global) | Grátis (orçamentos só com avisos não pagam nada) |
| Tópico SNS com o teu email | SNS, região `eu-north-1` (Estocolmo) | Grátis nesta escala (o plano gratuito inclui 1 000 emails por mês) |
| Alarme do tráfego de saída (`NetworkOut`) da EC2 do relay | CloudWatch, região `eu-north-1` | Grátis (o plano gratuito inclui 10 alarmes) |

### Euros e dólares

O AWS Budgets mostra e guarda os valores **sempre em dólares (USD)**, mesmo que pagues a fatura em euros. Para um orçamento de 10 €, põe **11 USD** (10 € são cerca de 11 a 12 USD; confirma o câmbio do dia e ajusta). Mais tarde podes mudar o valor sem apagar o orçamento.

## Parte 1: orçamento mensal (consola)

1. Entra na consola da AWS com a conta principal (ou um utilizador com acesso à faturação).
2. Na pesquisa do topo escreve **Budgets** e abre **Billing and Cost Management → Budgets**.
3. Carrega em **Create budget**.
4. Em **Budget setup** escolhe **Customize (advanced)** e, em **Budget types**, **Cost budget**. Carrega em **Next**.
5. Em **Details**:
   * **Budget name:** `sealer-mensal`
   * **Period:** `Monthly`
   * **Budget renewal type:** `Recurring budget`
   * **Start month:** o mês atual
   * **Budgeting method:** `Fixed`
   * **Enter your budgeted amount ($):** `11` (ver "Euros e dólares")
6. Em **Budget scope** deixa **All AWS services (Recommended)**. Deixa as opções avançadas como estão. Carrega em **Next**.
7. Em **Configure alerts** carrega em **Add an alert threshold** quatro vezes e preenche assim:

   | Alerta | Threshold | Trigger | Email recipients |
   | --- | --- | --- | --- |
   | 1 | `50` % of budgeted amount | **Actual** | o teu email |
   | 2 | `80` % of budgeted amount | **Actual** | o teu email |
   | 3 | `100` % of budgeted amount | **Actual** | o teu email |
   | 4 | `100` % of budgeted amount | **Forecasted** | o teu email |

   O alerta "Forecasted" (previsto) avisa quando a AWS prevê que o mês vai acabar acima do orçamento, antes de o gasto real lá chegar. A AWS só faz previsões depois de ter alguns dias de dados da conta; numa conta muito nova este aviso pode demorar a funcionar.
8. Carrega em **Next**. Em **Attach actions** não ponhas nada (as ações automáticas podem desligar serviços e não são precisas aqui). Carrega em **Next** e depois em **Create budget**.
9. Confirma na lista que aparece `sealer-mensal` com o valor certo. Os emails dos avisos vêm de `aws-budgets@costalerts.amazonaws.com`: se não chegarem, procura no spam e marca o endereço como seguro.

## Parte 2: alarme do tráfego de saída da EC2 (consola)

O tráfego que sai da EC2 para a Internet é o custo que mais pode crescer depressa (sobretudo o vídeo pelo relay). O plano gratuito da AWS inclui 100 GB de saída por mês, somados entre todos os serviços; acima disso paga-se por GB (vê o preço atual de `eu-north-1` na página de preços da EC2, em "Data Transfer").

### 2.1 Tópico de email (SNS)

1. Muda a região no topo da consola para **Europe (Stockholm) eu-north-1** (a mesma da EC2).
2. Abre **Simple Notification Service (SNS) → Topics → Create topic**.
3. **Type:** `Standard`; **Name:** `sealer-alarmes`. Carrega em **Create topic**.
4. Dentro do tópico, **Create subscription**: **Protocol** `Email`, **Endpoint** o teu email. Carrega em **Create subscription**.
5. Abre o email "AWS Notification - Subscription Confirmation" e carrega em **Confirm subscription**. Sem esta confirmação, o alarme não te consegue avisar.

### 2.2 Alarme

1. Ainda em `eu-north-1`, abre **CloudWatch → Alarms → All alarms → Create alarm**.
2. **Select metric → EC2 → Per-Instance Metrics**. Procura a linha com o ID da instância do relay e a métrica **NetworkOut**. Marca-a e carrega em **Select metric**.
   (O ID da instância está em **EC2 → Instances**, coluna "Instance ID", algo como `i-0abc...`.)
3. Em **Metric**:
   * **Statistic:** `Sum`
   * **Period:** `1 hour`
4. Em **Conditions**:
   * **Threshold type:** `Static`
   * **Whenever NetworkOut is...** `Greater` than `5000000000` (5 GB numa hora; a métrica está em bytes)
5. Em **Additional configuration**: **Datapoints to alarm** `1 out of 1`; **Missing data treatment** `Treat missing data as good (not breaching threshold)`. Carrega em **Next**.
6. Em **Notification**: **Alarm state trigger** `In alarm`; **Send a notification to the following SNS topic** → **Select an existing SNS topic** → `sealer-alarmes`. Carrega em **Next**.
7. **Alarm name:** `sealer-trafego-saida-1h`. **Next** e **Create alarm**.

**Como escolher o limite:** 5 GB numa hora é muito acima do uso normal com poucos testadores e chega para apanhar um abuso ou um erro cedo. Para ajustar, abre o gráfico da métrica `NetworkOut` (Sum, 1 hora) dos últimos dias e põe o limite em cerca de 3 a 5 vezes a hora mais alta que vires.

## Parte 3 (alternativa): os mesmos passos com a AWS CLI

Precisas da AWS CLI v2 configurada com um utilizador que possa usar Budgets, SNS, CloudWatch e EC2 (`aws configure`). Troca `O_TEU_EMAIL` pelo teu email.

### 3.1 Orçamento

```bash
EMAIL="O_TEU_EMAIL"
CONTA=$(aws sts get-caller-identity --query Account --output text)

cat > /tmp/orcamento.json <<'EOF'
{
  "BudgetName": "sealer-mensal",
  "BudgetLimit": { "Amount": "11", "Unit": "USD" },
  "TimeUnit": "MONTHLY",
  "BudgetType": "COST"
}
EOF

cat > /tmp/avisos.json <<EOF
[
  { "Notification": { "NotificationType": "ACTUAL", "ComparisonOperator": "GREATER_THAN", "Threshold": 50, "ThresholdType": "PERCENTAGE" },
    "Subscribers": [ { "SubscriptionType": "EMAIL", "Address": "$EMAIL" } ] },
  { "Notification": { "NotificationType": "ACTUAL", "ComparisonOperator": "GREATER_THAN", "Threshold": 80, "ThresholdType": "PERCENTAGE" },
    "Subscribers": [ { "SubscriptionType": "EMAIL", "Address": "$EMAIL" } ] },
  { "Notification": { "NotificationType": "ACTUAL", "ComparisonOperator": "GREATER_THAN", "Threshold": 100, "ThresholdType": "PERCENTAGE" },
    "Subscribers": [ { "SubscriptionType": "EMAIL", "Address": "$EMAIL" } ] },
  { "Notification": { "NotificationType": "FORECASTED", "ComparisonOperator": "GREATER_THAN", "Threshold": 100, "ThresholdType": "PERCENTAGE" },
    "Subscribers": [ { "SubscriptionType": "EMAIL", "Address": "$EMAIL" } ] }
]
EOF

aws budgets create-budget --account-id "$CONTA" \
  --budget file:///tmp/orcamento.json \
  --notifications-with-subscribers file:///tmp/avisos.json

# Confirmar
aws budgets describe-budget --account-id "$CONTA" --budget-name sealer-mensal
```

Para mudar o valor mais tarde (por exemplo para 22 USD):

```bash
aws budgets update-budget --account-id "$CONTA" --new-budget \
  '{"BudgetName":"sealer-mensal","BudgetLimit":{"Amount":"22","Unit":"USD"},"TimeUnit":"MONTHLY","BudgetType":"COST"}'
```

### 3.2 Tópico de email e alarme

```bash
REGIAO=eu-north-1

# ID da instancia do relay (ver a coluna Name)
aws ec2 describe-instances --region $REGIAO \
  --query "Reservations[].Instances[].[InstanceId,State.Name,Tags[?Key=='Name']|[0].Value]" --output table
INSTANCIA="i-COLOCA_AQUI_O_ID"

TOPICO=$(aws sns create-topic --name sealer-alarmes --region $REGIAO --query TopicArn --output text)
aws sns subscribe --topic-arn "$TOPICO" --protocol email --notification-endpoint "$EMAIL" --region $REGIAO
# Abre o email da AWS e confirma a subscricao antes de continuar.

aws cloudwatch put-metric-alarm --region $REGIAO \
  --alarm-name sealer-trafego-saida-1h \
  --alarm-description "Mais de 5 GB a sair da EC2 do relay numa hora" \
  --namespace AWS/EC2 --metric-name NetworkOut \
  --dimensions Name=InstanceId,Value="$INSTANCIA" \
  --statistic Sum --period 3600 --evaluation-periods 1 --datapoints-to-alarm 1 \
  --threshold 5000000000 --comparison-operator GreaterThanThreshold \
  --treat-missing-data notBreaching \
  --alarm-actions "$TOPICO"

# Confirmar (StateValue deve ficar OK ao fim de uma hora)
aws cloudwatch describe-alarms --region $REGIAO --alarm-names sealer-trafego-saida-1h \
  --query "MetricAlarms[].[AlarmName,StateValue,Threshold]" --output table
```

Para testar o email do alarme sem esperar por tráfego:

```bash
aws cloudwatch set-alarm-state --region $REGIAO --alarm-name sealer-trafego-saida-1h \
  --state-value ALARM --state-reason "Teste do aviso por email"
```

O alarme volta sozinho a `OK` na avaliação seguinte.

## Notas

* Se trocares de instância EC2 (instância nova = ID novo), o alarme da parte 2 tem de ser recriado com o ID novo. O orçamento não muda.
* O relay das chamadas tem também um travão próprio do tráfego do mês (`CALL_MONTHLY_EGRESS_GB` no `setup-ec2.sh`); o alarme da AWS é uma segunda rede de segurança, independente do relay.
* Opcional: a AWS também permite um alarme do CloudWatch sobre a fatura estimada (`EstimatedCharges`), mas só na região `us-east-1` e depois de ligar **Billing preferences → Receive CloudWatch billing alerts**. Com o orçamento da parte 1 não é preciso.
