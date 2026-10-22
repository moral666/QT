"""Leitura da AWS com cache, para gastar o mínimo de chamadas.

Cadência (com atualização a cada 60 s):
- describe_instance_status ........ cada atualização (API EC2, sem custo)
- métricas do agente (período 60 s) cada atualização (GetMetricStatistics, dentro do 1M grátis/mês)
- métricas EC2 (período 300 s) .... no máximo a cada 5 min (mais seria repetir o mesmo ponto)
- lista de instâncias ............. no máximo a cada 10 min
- Cost Explorer ................... no máximo a cada 6 h, cache em disco (cada pedido custa 0,01 USD)

Não usamos GetMetricData de propósito: não entra no plano gratuito do CloudWatch.
"""
from __future__ import annotations

import time
from datetime import date, datetime, timedelta, timezone
from typing import Any

from . import config

try:
    import boto3
    from botocore.config import Config as BotoConfig
    from botocore.exceptions import BotoCoreError, ClientError
except ImportError:  # a app continua a funcionar para o site e os badges
    boto3 = None
    BotoConfig = None
    BotoCoreError = ClientError = Exception

INSTANCES_TTL = 600
NATIVE_TTL = 300
COST_TTL = 6 * 3600

NATIVE_SPECS = (
    ("cpu", "CPUUtilization", "Average"),
    ("network_in", "NetworkIn", "Sum"),
    ("network_out", "NetworkOut", "Sum"),
)
CUSTOM_SPECS = (
    ("memory", "MemoryUsedPercent", "Average"),
    ("disk", "DiskUsedPercent", "Average"),
    ("connections", "ActiveTcpConnections", "Maximum"),
    ("errors", "RelayErrors", "Sum"),
    ("relay_up", "RelayServiceUp", "Minimum"),
)


def _short(exc: Exception, n: int = 220) -> str:
    return str(exc)[:n]


def friendly_error(msg: str, profile: str = "default") -> str:
    """Traduz os erros mais comuns do boto3 numa instrução clara."""
    low = msg.lower()
    if "could not be found" in low and "profile" in low:
        return f"O perfil AWS '{profile}' não existe neste computador. Corre: aws configure --profile {profile}"
    if "unable to locate credentials" in low:
        return "Não há credenciais AWS. Corre: aws configure (usa a chave do utilizador sealer-monitor)."
    if "unauthorizedoperation" in low or "accessdenied" in low or "not authorized" in low:
        return "Faltam permissões IAM ao utilizador sealer-monitor. " + msg[:200]
    if "could not connect" in low or "connect timeout" in low or "name resolution" in low:
        return "Sem ligação à AWS (internet ou firewall). " + msg[:160]
    if "signaturedoesnotmatch" in low or "invalidclienttokenid" in low:
        return "A chave AWS é inválida ou foi desativada. Cria uma nova no IAM e corre aws configure."
    return msg


def last_value(points: list[dict[str, Any]], key: str) -> float | None:
    for p in reversed(points):
        if p.get(key) is not None:
            return float(p[key])
    return None


class AWSMonitor:
    """Adaptador da API AWS. As credenciais vêm do perfil do AWS CLI."""

    def __init__(self, region: str, profile: str, namespace: str, instance_id: str = ""):
        if boto3 is None:
            raise RuntimeError("A biblioteca boto3 não está instalada.")
        self.region = region.strip() or "eu-north-1"
        self.profile = profile.strip() or None
        self.namespace = namespace.strip() or "Sealer/Relay"
        self.instance_id = instance_id.strip()
        retry = BotoConfig(retries={"max_attempts": 3, "mode": "standard"}, connect_timeout=5, read_timeout=10)
        session = boto3.Session(profile_name=self.profile, region_name=self.region)
        self.ec2 = session.client("ec2", config=retry)
        self.cw = session.client("cloudwatch", config=retry)
        self.ce = session.client("ce", region_name="us-east-1", config=retry)  # Cost Explorer só existe aqui
        self._instances: tuple[float, list[dict[str, Any]]] | None = None
        self._native: tuple[float, str, dict[str, Any]] | None = None
        self.calls = 0  # contador de chamadas, mostrado na interface

    # --- EC2 -------------------------------------------------------------
    def list_instances(self, force: bool = False) -> list[dict[str, Any]]:
        if not force and self._instances and time.monotonic() - self._instances[0] < INSTANCES_TTL:
            return self._instances[1]
        rows = []
        kwargs: dict[str, Any] = {"Filters": [{"Name": "instance-state-name", "Values": ["pending", "running", "stopping", "stopped"]}]}
        if self.instance_id:
            kwargs = {"InstanceIds": [self.instance_id]}
        for page in self.ec2.get_paginator("describe_instances").paginate(**kwargs):
            self.calls += 1
            for reservation in page.get("Reservations", []):
                for inst in reservation.get("Instances", []):
                    tags = {t.get("Key"): t.get("Value") for t in inst.get("Tags", [])}
                    rows.append({
                        "id": inst.get("InstanceId", ""),
                        "name": tags.get("Name", "(sem nome)"),
                        "state": inst.get("State", {}).get("Name", "unknown"),
                        "type": inst.get("InstanceType", "—"),
                        "az": inst.get("Placement", {}).get("AvailabilityZone", "—"),
                        "public_ip": inst.get("PublicIpAddress", "—"),
                    })
        rows.sort(key=lambda x: (x["state"] != "running", x["name"].lower(), x["id"]))
        self._instances = (time.monotonic(), rows)
        return rows

    def pick_instance(self, instances: list[dict[str, Any]]) -> dict[str, Any]:
        if not instances:
            raise RuntimeError(f"Não encontrei instâncias EC2 em {self.region}. Confirma a região e as permissões.")
        if self.instance_id:
            for x in instances:
                if x["id"] == self.instance_id:
                    return x
            raise RuntimeError(f"A instância {self.instance_id} não existe em {self.region} ou não tens permissão.")
        running = [x for x in instances if x["state"] == "running"] or instances
        return next((x for x in running if "sealer" in f"{x['name']} {x['id']}".lower() or "qt-relay" in x["name"].lower()), running[0])

    # --- CloudWatch ------------------------------------------------------
    def _metric(self, instance_id: str, name: str, namespace: str, stat: str, period: int) -> list[dict[str, Any]]:
        end = datetime.now(timezone.utc)
        self.calls += 1
        res = self.cw.get_metric_statistics(
            Namespace=namespace, MetricName=name,
            Dimensions=[{"Name": "InstanceId", "Value": instance_id}],
            StartTime=end - timedelta(minutes=60), EndTime=end,
            Period=period, Statistics=[stat],
        )
        return sorted(res.get("Datapoints", []), key=lambda d: d["Timestamp"])

    def _native_metrics(self, instance_id: str, alerts: list) -> dict[str, Any]:
        if self._native and self._native[1] == instance_id and time.monotonic() - self._native[0] < NATIVE_TTL:
            return self._native[2]
        out = {}
        for label, metric, stat in NATIVE_SPECS:
            try:
                pts = self._metric(instance_id, metric, "AWS/EC2", stat, 300)
                out[label] = {"value": last_value(pts, stat), "points": [p[stat] for p in pts]}
            except (BotoCoreError, ClientError) as exc:
                out[label] = {"value": None, "points": []}
                alerts.append(("warn", f"Métrica AWS indisponível: {metric}", _short(exc)))
        self._native = (time.monotonic(), instance_id, out)
        return out

    def _custom_metrics(self, instance_id: str) -> dict[str, Any]:
        out = {}
        for label, metric, stat in CUSTOM_SPECS:
            try:
                pts = self._metric(instance_id, metric, self.namespace, stat, 60)
                out[label] = {"value": last_value(pts, stat), "points": [p[stat] for p in pts]}
            except (BotoCoreError, ClientError) as exc:
                out[label] = {"value": None, "points": [], "error": _short(exc, 180)}
        return out

    # --- Custos ----------------------------------------------------------
    def cost_month_to_date(self, force: bool = False) -> dict[str, Any]:
        """Custo do mês separado por tipo de registo, num só pedido.

        O UnblendedCost total já vem com os créditos descontados (fica ~0 enquanto
        houver créditos). Agrupando por RECORD_TYPE vê-se o uso real e os créditos.
        """
        key = f"cost2:{self.profile or 'default'}"
        cache = config.load_cache()
        hit = cache.get(key)
        today = date.today()
        if (not force and isinstance(hit, dict) and hit.get("month") == today.strftime("%Y-%m")
                and time.time() - hit.get("ts", 0) < COST_TTL):
            return hit
        start = today.replace(day=1).isoformat()
        end = (today + timedelta(days=1)).isoformat()  # o fim é exclusivo
        self.calls += 1
        res = self.ce.get_cost_and_usage(TimePeriod={"Start": start, "End": end}, Granularity="MONTHLY",
                                         Metrics=["UnblendedCost"],
                                         GroupBy=[{"Type": "DIMENSION", "Key": "RECORD_TYPE"}])
        obj = summarize_cost(res)
        obj.update(ts=time.time(), month=today.strftime("%Y-%m"))
        cache.pop(f"cost:{self.profile or 'default'}", None)  # formato antigo
        cache[key] = obj
        config.save_cache(cache)
        return obj

    # --- Tudo junto ------------------------------------------------------
    def snapshot(self, force: bool = False) -> dict[str, Any]:
        alerts: list[tuple[str, str, str]] = []
        instances = self.list_instances(force)
        inst = self.pick_instance(instances)
        iid = inst["id"]
        data: dict[str, Any] = {"instance": inst, "region": self.region, "alerts": alerts,
                                "collected_at": datetime.now().astimezone().strftime("%H:%M:%S")}
        try:
            self.calls += 1
            st = self.ec2.describe_instance_status(InstanceIds=[iid], IncludeAllInstances=True).get("InstanceStatuses", [])
            if st:
                inst["state"] = st[0].get("InstanceState", {}).get("Name", inst["state"])
                data["system_status"] = str(st[0].get("SystemStatus", {}).get("Status") or "unknown").lower()
                data["instance_status"] = str(st[0].get("InstanceStatus", {}).get("Status") or "unknown").lower()
            else:
                data["system_status"] = data["instance_status"] = "initializing"
        except (BotoCoreError, ClientError) as exc:
            data["system_status"] = data["instance_status"] = "unknown"
            alerts.append(("warn", "Não foi possível ler os status checks EC2", _short(exc)))

        data["native"] = self._native_metrics(iid, alerts) if not force else self._refresh_native(iid, alerts)
        data["custom"] = self._custom_metrics(iid)
        try:
            data["cost"] = self.cost_month_to_date()  # nunca forçado: cada pedido custa 0,01 USD
        except Exception as exc:  # Cost Explorer desligado ou sem permissão
            data["cost"] = {"amount": None, "unit": "", "error": _short(exc, 240)}
        data["calls"] = self.calls
        evaluate(data)
        return data

    def _refresh_native(self, iid: str, alerts: list) -> dict[str, Any]:
        self._native = None
        return self._native_metrics(iid, alerts)


CREDIT_TYPES = {"Credit", "Refund", "BundledDiscount", "DiscountedUsage", "SavingsPlanNegation"}


def summarize_cost(res: dict[str, Any]) -> dict[str, Any]:
    """usage = tudo menos créditos/reembolsos; credits = créditos (negativo); net = a pagar."""
    usage = credits = 0.0
    unit = ""
    found = False
    for period in res.get("ResultsByTime", []):
        for g in period.get("Groups", []):
            m = g.get("Metrics", {}).get("UnblendedCost", {})
            if m.get("Amount") is None:
                continue
            found = True
            unit = m.get("Unit", unit)
            amount = float(m["Amount"])
            if (g.get("Keys") or [""])[0] in CREDIT_TYPES:
                credits += amount
            else:
                usage += amount
    if not found:
        return {"amount": None, "usage": None, "credits": None, "unit": unit}
    net = usage + credits
    clean = lambda v: 0.0 if abs(v) < 0.005 else round(v, 2)  # evita "-0.00"
    return {"amount": clean(net), "usage": clean(usage), "credits": clean(credits), "unit": unit or "USD"}


def evaluate(data: dict[str, Any]) -> None:
    """Acrescenta os alertas de saúde a data['alerts'] e define data['tone']."""
    alerts = data["alerts"]
    inst = data["instance"]
    native, custom = data.get("native", {}), data.get("custom", {})
    cpu = native.get("cpu", {}).get("value")
    mem = custom.get("memory", {}).get("value")
    disk = custom.get("disk", {}).get("value")
    relay = custom.get("relay_up", {}).get("value")
    errors = custom.get("errors", {}).get("value")

    if inst["state"] != "running":
        alerts.append(("bad", f"A instância está {inst['state']}", "O relay não está disponível."))
    for key, name in (("system_status", "system"), ("instance_status", "instance")):
        if data.get(key) not in ("ok", "initializing"):
            alerts.append(("bad", f"Status check {name} não está OK", f"Estado: {data.get(key)}"))
    if cpu is not None and cpu >= 85:
        alerts.append(("warn", f"CPU elevada: {cpu:.1f}%", "Verifica a carga do relay e os processos."))
    if mem is not None and mem >= 90:
        alerts.append(("bad" if mem >= 96 else "warn", f"RAM elevada: {mem:.1f}%", "Pode causar lentidão ou falta de memória."))
    if disk is not None and disk >= 90:
        alerts.append(("bad" if disk >= 96 else "warn", f"Disco quase cheio: {disk:.1f}%", "Liberta espaço ou aumenta o volume."))
    if relay is not None and relay < 0.5:
        alerts.append(("bad", "O serviço do relay parece parado", "Confirma a unidade systemd qt-relay."))
    if errors:
        alerts.append(("warn", f"{errors:.0f} erro(s) no relay no último minuto", "Vê sudo journalctl -u qt-relay."))

    tones = {a[0] for a in alerts}
    data["tone"] = "bad" if "bad" in tones else ("warn" if "warn" in tones else "ok")
