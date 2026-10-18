//! Callee node kinds across the supported grammars.
//!
//! Regression guard for a bug where `maybe_edge` only matched `identifier` for
//! Python: an attribute call (`self._validate(x)`, `obj.helper(y)`) never
//! produced an edge, so the method layer of the call graph silently vanished —
//! `trace <method> -d callers` returned an empty tree while everything else
//! looked healthy.

use imc_parser::parse;

fn dsts(rel: &str, src: &str) -> Vec<String> {
    let p = parse(rel, src).expect("parse failed");
    p.edges
        .into_iter()
        .filter(|e| e.kind.as_str() == "call")
        .map(|e| e.dst)
        .collect()
}

#[test]
fn python_covers_plain_and_attribute_calls() {
    let src = "class T:\n    def m(self):\n        self._validate(1)\n        self.obj.helper(2)\n        plain(3)\n";
    let d = dsts("src/a.py", src);
    // A module-level `plain(3)` outside any def/class is intentionally not an
    // edge (no enclosing scope to attribute it to).
    for want in ["_validate", "helper", "plain"] {
        assert!(d.iter().any(|x| x == want), "missing edge to {want}: {d:?}");
    }
}

#[test]
fn rust_covers_field_expression_calls() {
    let src = "fn f(x: u32) {\n    self.helper(x);\n    engine.throttle(x);\n    spawn(x);\n}\n";
    let d = dsts("src/lib.rs", src);
    for want in ["helper", "throttle", "spawn"] {
        assert!(d.iter().any(|x| x == want), "missing edge to {want}: {d:?}");
    }
}

#[test]
fn go_covers_selector_calls() {
    let src = "func main() {\n\tsrv.Start()\n\tNewServer(8080)\n}\n";
    let d = dsts("cmd/main.go", src);
    for want in ["Start", "NewServer"] {
        assert!(d.iter().any(|x| x == want), "missing edge to {want}: {d:?}");
    }
}

#[test]
fn java_covers_method_invocation_and_creation() {
    let src = "import java.util.List;

public class OrderService {
    private final Calculator calc = new Calculator();

    public int total(List<Integer> items) {
        int sum = calc.add(1, 2);
        return sum;
    }
}
";
    let p = parse("src/OrderService.java", src).expect("parse failed");
    let kinds: Vec<&str> = p.symbols.iter().map(|s| s.kind.as_str()).collect();
    for want in ["class", "interface", "method"] {
        if want != "interface" {
            assert!(kinds.contains(&want), "missing {want}: {kinds:?}");
        }
    }
    assert!(
        p.symbols.iter().any(|s| s.name == "OrderService"),
        "class name: {:?}",
        p.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(p.symbols.iter().any(|s| s.name == "total"), "method total");

    let dsts: Vec<&str> = p
        .edges
        .iter()
        .filter(|e| e.kind.as_str() == "call")
        .map(|e| e.dst.as_str())
        .collect();
    for want in ["add", "Calculator"] {
        assert!(dsts.contains(&want), "missing edge to {want}: {dsts:?}");
    }
    assert!(
        p.edges.iter().any(|e| e.kind.as_str() == "import"),
        "import edge missing"
    );
}

#[test]
fn java_interface_and_enum_are_recognised() {
    let src = "public interface Repo<T> { T get(int id); }

enum Color { RED, GREEN; }

public record Point(int x, int y) {}
";
    let p = parse("src/Mixed.java", src).expect("parse failed");
    let names: Vec<&str> = p.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Repo"), "interface: {names:?}");
    assert!(names.contains(&"Color"), "enum: {names:?}");
    assert!(names.contains(&"Point"), "record: {names:?}");
    assert!(names.contains(&"get"), "interface method: {names:?}");
}

/// Module-level imports are attributed to the file, exactly like Python's
/// graph builder does with the file scope. Dropping them (the old behaviour)
/// meant `use`/`import` edges only existed inside function bodies.
#[test]
fn module_level_imports_are_recorded() {
    let py = parse("src/a.py", "import os\nfrom math import sqrt\n\ndef f():\n    return sqrt(4)\n")
        .expect("parse failed");
    let imports: Vec<&str> = py
        .edges
        .iter()
        .filter(|e| e.kind.as_str() == "import")
        .map(|e| e.src.as_str())
        .collect();
    assert!(imports.iter().any(|s| *s == "src/a.py"), "py imports: {imports:?}");

    let go = parse("cmd/main.go", "package main\n\nimport \"fmt\"\n").expect("parse failed");
    assert!(
        go.edges.iter().any(|e| e.kind.as_str() == "import"),
        "go import edge missing"
    );
}

#[test]
fn typescript_covers_property_access_calls() {
    let src = "export function f(){ api.client.fetch(1); helper(2); }\n";
    let d = dsts("src/a.ts", src);
    for want in ["fetch", "helper"] {
        assert!(d.iter().any(|x| x == want), "missing edge to {want}: {d:?}");
    }
}