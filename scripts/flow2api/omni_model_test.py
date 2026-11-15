"""Check public Omni names map to the new upstream family without importing runtime services."""
import ast
from pathlib import Path
from typing import Optional

root = Path(__file__).resolve().parents[2]
source = root / "flow2api-src/src/services/generation_handler.py"
tree = ast.parse(source.read_text(encoding="utf-8"))
catalog_nodes = []
for node in tree.body:
    if isinstance(node, ast.Assign) and any(getattr(t, "id", "") in ("MODEL_CONFIG", "MODEL_DESCRIPTIONS") for t in node.targets):
        catalog_nodes.append(node)
    elif isinstance(node, ast.FunctionDef) and (node.name.startswith("_make_") or node.name == "_apply_current_flow_model_catalog"):
        catalog_nodes.append(node)
    elif isinstance(node, ast.Expr) and isinstance(node.value, ast.Call) and getattr(node.value.func, "id", "") == "_apply_current_flow_model_catalog":
        catalog_nodes.append(node)
namespace = {"Dict": dict, "Any": object, "Optional": Optional}
exec(compile(ast.Module(body=catalog_nodes, type_ignores=[]), str(source), "exec"), namespace)
models = namespace["MODEL_CONFIG"]
for seconds in range(3, 16):
    for orientation in ("landscape", "portrait"):
        key = f"omni-1.2-flash-{seconds}s-{orientation}"
        if seconds > 10 and orientation == "landscape":
            assert key not in models
            continue
        assert models[key]["model_key"] == f"macaron_cookie_t2v_{seconds}s"
        assert models[key]["reference_model_key"] == f"macaron_cookie_r2v_{seconds}s"
assert models["omni-1.2-flash"]["reference_model_key"] == "macaron_cookie_r2v_8s"
assert "omni-1.1-flash" in models
for runtime in ("src-tauri/flow2api", "installed/flow2api"):
    target = root / runtime / "src/services/generation_handler.py"
    if target.exists():
        assert target.read_bytes() == source.read_bytes()
print("Omni 1.2 landscape/portrait model matrix and runtime copies: OK")
