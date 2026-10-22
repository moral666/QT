"""Abre a app offscreen com dados simulados e grava capturas das 3 páginas."""
import math, os, sys, threading, time
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
import tempfile; os.environ["HOME"] = tempfile.mkdtemp()
from sealer_control import aws, services, app as appmod
from tests.test_logic import REAL

OUT = os.environ.get("SHOTS", "/tmp"); os.makedirs(OUT, exist_ok=True)
main_thread = threading.get_ident()
seen_threads = []

def fake_site(url, slow, timeout=5):
    seen_threads.append(threading.get_ident())
    return {"tone": "ok", "title": "Online", "detail": "HTTP 200 · 182 ms", "code": 200, "ms": 182.0, "ts": time.time()}
services.check_site = fake_site
services.fetch_estado = lambda url, timeout=5: dict(REAL, atualizado=time.time() - 20, ultima_ida_medicao=time.time() - 20, ultima_medicao=time.time() - 8280)

class FakeMon:
    def __init__(self, *a): self.calls = 0
    def snapshot(self, force=False):
        self.calls += 12
        pts = lambda base, amp, n=60: [base + amp * math.sin(i / 6) + (i % 7) * amp / 10 for i in range(n)]
        d = {"instance": {"id": "i-0a1b2c3d4e5f67890", "name": "qt-relay", "state": "running", "type": "t3.small",
                          "az": "eu-north-1b", "public_ip": "13.51.46.213"},
             "region": "eu-north-1", "alerts": [], "collected_at": time.strftime("%H:%M:%S"),
             "system_status": "ok", "instance_status": "ok", "calls": self.calls,
             "native": {"cpu": {"value": 0.3, "points": pts(0.3, 0.1, 12)}, "network_in": {"value": 18500.0},
                        "network_out": {"value": 24100.0}},
             "custom": {"memory": {"value": 41.2, "points": pts(40, 1.5)}, "disk": {"value": 23.8, "points": pts(23.7, 0.05)},
                        "connections": {"value": 7.0, "points": pts(6, 2)}, "errors": {"value": 0.0, "points": []},
                        "relay_up": {"value": 1.0, "points": []}},
             "cost": {"amount": 0.0, "usage": 3.47, "credits": -3.47, "unit": "USD", "ts": time.time() - 3600}}
        aws.evaluate(d)
        return d
aws.AWSMonitor = FakeMon
aws.boto3 = object()

from PySide6.QtCore import QTimer
from PySide6.QtWidgets import QApplication
app = QApplication(sys.argv)
app.setStyleSheet(appmod.QSS)
win = appmod.MainWindow()
win.resize(1280, 900)
win.show()
win.badges.user.setText("qt1:Ab3dE5fG7hJ9kLmN2pQ")
win.badges._write("\n[10:41:02] Founder → qt1:Ab3dE5fG7hJ9kLmN2pQ", appmod.C["accent"])
win.badges._write("$ cargo run --release -p qt_server --example badge -- dar --key …/badge-emissor.key --db …/badge-emissor.db --user qt1:Ab3dE5fG7hJ9kLmN2pQ --badge founder", appmod.C["faint"])
win.badges._write("  Finished `release` profile [optimized] target(s) in 0.21s", appmod.C["muted"])
win.badges._write("✓ Badge atualizado. (1.4 s)", appmod.C["green"])

def shoot():
    for i, name in enumerate(("relay", "site", "badges")):
        win.nav_group.button(i).click()
        app.processEvents()
        page = win.stack.currentWidget().widget()
        win.grab().save(os.path.join(OUT, f"{name}.png"))
        page.grab().save(os.path.join(OUT, f"{name}_full.png"))
    print("callbacks in main thread:", threading.get_ident() == main_thread, "worker threads used:", any(t != main_thread for t in seen_threads))
    print("global:", win.global_text.text(), {k: v for k, v in win.tones.items()})
    app.quit()
QTimer.singleShot(2500, shoot)
app.exec()
