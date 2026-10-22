import json, os, tempfile
from sealer_control import services, badges, aws, reports

REAL = json.loads('{"entrega_media_ms":null,"entrega_mediana_ms":null,"entrega_p95_ms":null,"mensagens":0,"janela_minutos":60,"espera_maxima_s":60,"atualizado":1792637539,"ultima_media_ms":2920,"ultima_mediana_ms":2713,"ultima_medicao":1792629259,"ida_media_ms":46,"ida_mediana_ms":32,"ida_p95_ms":139,"ida_amostras":11,"ultima_ida_ms":46,"ultima_ida_mediana_ms":32,"ultima_ida_medicao":1792637539}')

def test_estado_real():
    e = services.interpret_estado(REAL, now=1792637539 + 20)
    assert e["ida"] == 32 and e["tone"] == "ok"
    assert e["entrega"] == 2713 and not e["entrega_live"] and e["entrega_stale"]
    assert e["total"] is None and e["site_total"] == 2745   # o valor que o site mostrava
    assert services.age_text(e["entrega_age"]) == "há 2 h 18 min"

def test_estado_live_and_stale_relay():
    js = dict(REAL, entrega_mediana_ms=141, mensagens=20)
    e = services.interpret_estado(js, now=REAL["atualizado"] + 30)
    assert e["entrega_live"] and e["total"] == 173
    e = services.interpret_estado(REAL, now=REAL["atualizado"] + 900)
    assert e["relay_stale"] and e["tone"] == "bad"

def test_validate_user():
    assert badges.validate_user("qt1:") is not None
    assert badges.validate_user("abc") is not None
    assert badges.validate_user("qt1:abc def ghi") is not None
    assert badges.validate_user("qt1:x; rm -rf ~") is not None
    assert badges.validate_user("qt1:Ab3dE5fG7hJ9kL") is None

def test_build_command_no_shell(tmp_path=None):
    d = tempfile.mkdtemp()
    key = os.path.join(d, "SENSIVEL ", "k.key"); os.makedirs(os.path.dirname(key)); open(key, "w").close()
    cfg = {"repo_path": d, "badge_key": key, "badge_db": key + ".db"}
    try:
        cmd = badges.build_command(cfg, "qt1:Ab3dE5fG7hJ9kL", "founder")
    except ValueError as exc:
        assert "cargo" in str(exc)  # sem Rust neste ambiente
        return
    assert cmd.args[-4:] == ["--user", "qt1:Ab3dE5fG7hJ9kL", "--badge", "founder"]
    assert key in cmd.args  # espaço no caminho passa intacto, sem aspas
    for bad in ("hacker", "founder; ls"):
        try: badges.build_command(cfg, "qt1:Ab3dE5fG7hJ9kL", bad); assert False
        except ValueError: pass

def test_evaluate():
    d = {"instance": {"state": "running"}, "system_status": "ok", "instance_status": "ok", "alerts": [],
         "native": {"cpu": {"value": 3.0}}, "custom": {"memory": {"value": 97.0}, "relay_up": {"value": 1.0}}}
    aws.evaluate(d)
    assert d["tone"] == "bad" and any("RAM" in a[1] for a in d["alerts"])
    d = {"instance": {"state": "running"}, "system_status": "ok", "instance_status": "initializing", "alerts": [], "native": {}, "custom": {}}
    aws.evaluate(d); assert d["tone"] == "ok"

def test_cost_with_credits():
    res = {"ResultsByTime": [{"Groups": [
        {"Keys": ["Usage"], "Metrics": {"UnblendedCost": {"Amount": "3.4712", "Unit": "USD"}}},
        {"Keys": ["Tax"], "Metrics": {"UnblendedCost": {"Amount": "0.0", "Unit": "USD"}}},
        {"Keys": ["Credit"], "Metrics": {"UnblendedCost": {"Amount": "-3.4731", "Unit": "USD"}}}]}]}
    c = aws.summarize_cost(res)
    assert c["usage"] == 3.47 and c["credits"] == -3.47 and c["amount"] == 0.0  # nunca "-0.00"
    assert f"{c['amount']:.2f}" == "0.00"
    assert aws.summarize_cost({"ResultsByTime": [{"Groups": []}]})["usage"] is None

def test_reports_commands_and_parsing():
    assert reports.validate_relay_id("ab" * 32) is None
    for bad in ("", "ab" * 31, "zz" * 32, "ab" * 32 + "; ls"):
        assert reports.validate_relay_id(bad) is not None
    d = tempfile.mkdtemp()
    key = os.path.join(d, "SENSIVEL ", "operador.key"); os.makedirs(os.path.dirname(key)); open(key, "w").close()
    cfg = {"repo_path": d, "operator_key": key}
    try:
        cmd = reports.build_command(cfg, "banir", user="AB" * 32)
    except ValueError as exc:
        assert "cargo" in str(exc)  # sem Rust neste ambiente
    else:
        assert cmd.args[-2:] == ["--user", "ab" * 32] and key in cmd.args
        assert reports.build_command(cfg, "listar").args[-1] == "--json"
        assert reports.build_command(cfg, "apagar", report_id=7).args[-2:] == ["--id", "7"]
        for kw in ({"user": "ab; rm"}, {}):
            try: reports.build_command(cfg, "banir", **kw); assert False
            except ValueError: pass
        try: reports.build_command(cfg, "rm"); assert False
        except ValueError: pass
    got = reports.parse_reports('[{"id": 3, "recebida_em_ms": 1, "denuncia": {"motivo": "spam"}}, {"x": 1}]')
    assert [r["id"] for r in got] == [3]
    assert reports.parse_bans('["' + "ab" * 32 + '", 5]') == ["ab" * 32]


if __name__ == "__main__":
    for n, f in list(globals().items()):
        if n.startswith("test_"): f(); print("ok", n)
