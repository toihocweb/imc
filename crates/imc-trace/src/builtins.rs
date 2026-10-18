//! Trivial / language built-ins and standard library functions filtered out of
//! call traces in high-signal mode (shown again with `--all`).
//!
//! Port of `COMMON_BUILTINS`, `COMMON_POJO_ACCESSORS` and `is_trivial_accessor`
//! from `main/core/tracer.py` — membership is copied verbatim (same names, same
//! case, same duplicates); only the lookup mechanics changed (`HashSet` behind
//! a `OnceLock` instead of a Python `set`).

use std::collections::HashSet;
use std::sync::OnceLock;

/// Trivial / language built-ins and standard-library functions to filter out in
/// high-signal mode (can be shown with `--all`). 183 entries, 174 unique —
/// duplicated exactly as in the Python literal.
pub static COMMON_BUILTINS: &[&str] = &[
    // Python
    "print", "len", "range", "str", "int", "float", "bool", "list", "dict", "set",
    "tuple", "isinstance", "issubclass", "getattr", "setattr", "hasattr", "type",
    "id", "super", "enumerate", "zip", "map", "filter", "any", "all", "min", "max",
    "sum", "abs", "round", "open", "close", "read", "write", "append", "extend",
    "insert", "pop", "remove", "clear", "index", "count", "sort", "reverse", "copy",
    "get", "items", "keys", "values", "update", "split", "join", "strip", "lstrip",
    "rstrip", "replace", "lower", "upper", "startswith", "endswith", "encode",
    "decode", "format", "exit", "repr", "next", "iter", "callable", "vars", "dir",
    "sleep", "exists", "is_file", "is_dir", "resolve", "parent",
    // JS / TS
    "console", "log", "error", "warn", "info", "debug", "setTimeout", "setInterval",
    "clearTimeout", "clearInterval", "push", "forEach", "reduce", "findIndex",
    "includes", "indexOf", "slice", "splice", "trim", "toLowerCase", "toUpperCase",
    "toString", "parseInt", "parseFloat", "addEventListener", "removeEventListener",
    "preventDefault", "stopPropagation", "stringify", "parse", "keys", "values",
    "entries", "assign", "then", "catch", "finally",
    // Go / General
    "fmt", "Println", "Printf", "Sprintf", "make", "new", "cap", "panic", "recover",
    // Java & OOP Standard Library Builtins
    "now", "randomUUID", "UUID", "Instant", "Optional", "of", "ofNullable", "orElse",
    "orElseThrow", "orElseGet", "isPresent", "isEmpty", "requireNonNull", "stream",
    "collect", "toList", "toSet", "toMap", "equals", "hashCode", "toString", "valueOf",
    "builder", "build", "iterator", "hasNext", "prepareStatement", "close", "println",
    "printf", "format", "as", "wait", "notify", "notifyAll", "clone", "finalize",
    "getClass", "compareTo", "singletonList", "emptyList", "emptyMap", "emptySet",
    // Cypress / Test runner assertions that pollute traces
    "should", "and", "intercept", "visit", "click", "contains", "type", "clear", "blur",
    "wait", "eq", "first", "last", "find", "get", "typeIntoUi5Input", "clearUi5Input",
    "clickDropdownMenuItem",
];

/// Trivial getter/setter properties on POJOs, DTOs, Entities. Stored lowercase,
/// exactly as in Python; lookups lowercase the candidate first.
pub static COMMON_POJO_ACCESSORS: &[&str] = &[
    "getid", "setid", "getuuid", "setuuid", "getname", "setname", "getvalue", "setvalue",
    "getcreatedat", "setcreatedat", "getupdatedat", "setupdatedat", "getcreatedby", "setcreatedby",
    "getupdatedby", "setupdatedby", "getstatus", "setstatus", "gettype", "settype",
    "getdescription", "setdescription", "gettitle", "settitle", "getorder", "setorder",
    "getdisplayorder", "setdisplayorder", "getcode", "setcode", "getkey", "setkey",
    "getmessage", "setmessage", "geterror", "seterror", "getdata", "setdata",
    "getresult", "setresult", "getpayload", "setpayload", "getstate", "setstate",
    "getpath", "setpath", "geturl", "seturl", "getcount", "setcount", "getlength",
    "getsize", "gettime", "settime", "gettimestamp", "settimestamp", "getdate", "setdate",
    "gettranslationnamekey", "settranslationnamekey", "getispartnercontext", "setispartnercontext",
    "isvalid", "isdirty", "issubmitting", "isloading", "iserror", "issuccess", "isempty",
];

fn builtin_set() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| COMMON_BUILTINS.iter().copied().collect())
}

fn accessor_set() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| COMMON_POJO_ACCESSORS.iter().copied().collect())
}

/// `name in COMMON_BUILTINS` — case-sensitive, exactly like the Python check.
pub fn is_builtin(name: &str) -> bool {
    builtin_set().contains(name)
}

/// Detect trivial POJO/DTO property getters and setters.
/// `if not name: return False` + `name.lower() in COMMON_POJO_ACCESSORS`.
pub fn is_trivial_accessor(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    accessor_set().contains(name.to_lowercase().as_str())
}

/// Should this callee name be dropped from a trace? `show_all` (`--all`)
/// disables all filtering, matching `trace_callees(show_all=...)`.
pub fn should_filter(name: &str, show_all: bool) -> bool {
    should_filter_in(name, show_all, "x.py")
}

/// Same filter, but aware of the language of the file making the call.
///
/// `COMMON_BUILTINS` is Python's builtin list, kept verbatim for Python parity
/// (`tests` assert the 183 entries). Applying it to every language silently
/// deleted real calls in other grammars: a Java `calculator.sum(items)` or
/// `round(amount)` disappeared from the trace because Python has builtins by
/// those names. POJO accessors stay filtered everywhere — `getName`/`setAge`
/// are Java/.NET idiom, not Python's.
pub fn should_filter_in(name: &str, show_all: bool, caller_file: &str) -> bool {
    if show_all {
        return false;
    }
    if is_trivial_accessor(name) {
        return true;
    }
    let caller_is_python = caller_file.ends_with(".py") || caller_file.ends_with(".pyi");
    caller_is_python && is_builtin(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_contain_verbatim_entries() {
        for name in [
            "print", "len", "range", "isinstance", "dir",           // Python
            "setTimeout", "forEach", "toLowerCase", "stringify",    // JS / TS
            "fmt", "Println", "make", "panic",                       // Go / General
            "randomUUID", "orElseThrow", "prepareStatement", "getClass", // Java / OOP
            "should", "intercept", "typeIntoUi5Input", "clickDropdownMenuItem", // Cypress
        ] {
            assert!(is_builtin(name), "{name} should be a builtin");
        }
        assert!(!is_builtin("run_find"));
        assert!(!is_builtin("createPhase"));
        assert!(!is_builtin("handler"));
    }

    #[test]
    fn builtins_sizes_match_python_literal() {
        assert_eq!(COMMON_BUILTINS.len(), 183, "token count drifted from Python");
        assert_eq!(builtin_set().len(), 174, "unique builtin count drifted");
        assert_eq!(COMMON_POJO_ACCESSORS.len(), 69);
        assert_eq!(accessor_set().len(), 69);
    }

    #[test]
    fn builtin_lookup_is_case_sensitive() {
        // Python checks `name in COMMON_BUILTINS` verbatim.
        assert!(is_builtin("Println"));
        assert!(is_builtin("type"));
        assert!(!is_builtin("PRINT"));
        assert!(!is_builtin("printlnx"));
    }

    #[test]
    fn trivial_accessors_are_case_insensitive() {
        assert!(is_trivial_accessor("getName"));
        assert!(is_trivial_accessor("GetName"));
        assert!(is_trivial_accessor("getname"));
        assert!(is_trivial_accessor("isSubmitting"));
        assert!(is_trivial_accessor("setTranslationNameKey"));
        assert!(!is_trivial_accessor("getCustomerReport"));
        assert!(!is_trivial_accessor(""));
    }

    #[test]
    fn should_filter_respects_show_all() {
        assert!(should_filter("print", false));
        assert!(should_filter("getName", false));
        assert!(!should_filter("print", true));
        assert!(!should_filter("getName", true));
        assert!(!should_filter("run_find", false));
    }

    /// Regression: `should_filter` is Python's builtin list, but it was applied
    /// to callers of every language. A Java `calculator.sum(items)` and a
    /// `round(amount)` call disappeared from the default trace because Python
    /// has builtins of those names — silently truncating a call chain.
    #[test]
    fn python_builtin_list_only_applies_to_python_callers() {
        for name in ["sum", "round", "format", "map", "filter", "len", "list"] {
            assert!(
                should_filter_in(name, false, "main/cli/main.py"),
                "{name} must still be filtered for a Python caller"
            );
            assert!(
                !should_filter_in(name, false, "src/main/java/com/shop/OrderService.java"),
                "{name} must NOT be filtered for a Java caller"
            );
            assert!(
                !should_filter_in(name, false, "src/payments/ledger.ts"),
                "{name} must NOT be filtered for a TypeScript caller"
            );
            assert!(
                !should_filter_in(name, false, "src/engine.rs"),
                "{name} must NOT be filtered for a Rust caller"
            );
            assert!(
                !should_filter_in(name, false, "cmd/server/main.go"),
                "{name} must NOT be filtered for a Go caller"
            );
        }
    }

    /// POJO accessors are Java/.NET idiom, not Python's — they stay filtered
    /// regardless of caller language.
    #[test]
    fn pojo_accessors_stay_filtered_everywhere() {
        // The list is a fixed set, not a get*/set* prefix rule: `getname`,
        // `setvalue`, `getstatus` are in it, `setAge` is not.
        for file in ["a.py", "a.java", "a.ts", "a.rs", "a.go"] {
            assert!(should_filter_in("getName", false, file), "getName {file}");
            assert!(should_filter_in("setValue", false, file), "setValue {file}");
            assert!(should_filter_in("GETSTATUS", false, file), "GETSTATUS {file}");
        }
    }

    /// `--all` still bypasses everything, in every language.
    #[test]
    fn show_all_bypasses_language_aware_filter() {
        assert!(!should_filter_in("sum", true, "a.py"));
        assert!(!should_filter_in("getName", true, "a.java"));
    }
}
