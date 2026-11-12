use crate::scan::{Keys, Spec, PLAIN};

const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if",
    "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static",
    "struct", "super", "trait", "type", "unsafe", "use", "where", "while", "macro_rules",
];
const RUST_TY: &[&str] = &[
    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64", "bool", "char",
    "str", "String", "Vec", "Option", "Result", "Box",
];
const BOOLS: &[&str] = &["true", "false", "null", "nil", "None", "True", "False", "undefined", "NaN", "Some", "Ok", "Err"];

const JS_KW: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do",
    "else", "export", "extends", "finally", "for", "from", "function", "if", "import", "in", "instanceof", "let", "new",
    "of", "return", "static", "super", "switch", "this", "throw", "try", "typeof", "var", "void", "while", "with",
    "yield", "interface", "type", "enum", "implements", "namespace", "readonly", "public", "private", "protected",
    "declare", "abstract", "as", "keyof", "get", "set",
];
const JS_TY: &[&str] =
    &["string", "number", "boolean", "any", "unknown", "never", "object", "symbol", "bigint", "Promise", "Array", "Map", "Set"];

const PY_KW: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except",
    "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
    "return", "try", "while", "with", "yield", "self", "match", "case",
];
const PY_TY: &[&str] = &["int", "str", "float", "bool", "list", "dict", "set", "tuple", "bytes", "object"];

const SH_KW: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function",
    "select", "return", "exit", "local", "export", "readonly", "declare", "set", "unset", "source", "alias", "break",
    "continue", "trap", "shift",
];

const C_KW: &[&str] = &[
    "auto", "break", "case", "const", "continue", "default", "do", "else", "enum", "extern", "for", "goto", "if",
    "inline", "register", "return", "sizeof", "static", "struct", "switch", "typedef", "union", "volatile", "while",
    "class", "namespace", "template", "typename", "public", "private", "protected", "virtual", "override", "new",
    "delete", "using", "try", "catch", "throw", "constexpr", "this", "operator", "final", "noexcept", "explicit",
];
const C_TY: &[&str] = &[
    "int", "char", "short", "long", "float", "double", "void", "unsigned", "signed", "bool", "size_t", "uint8_t",
    "uint16_t", "uint32_t", "uint64_t", "int8_t", "int16_t", "int32_t", "int64_t", "auto", "string", "vector",
];

const GO_KW: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go",
    "goto", "if", "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type",
    "var",
];
const GO_TY: &[&str] = &[
    "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32", "uint64", "float32", "float64",
    "string", "bool", "byte", "rune", "error", "any",
];

const JAVA_KW: &[&str] = &[
    "abstract", "assert", "break", "case", "catch", "class", "const", "continue", "default", "do", "else", "enum",
    "extends", "final", "finally", "for", "goto", "if", "implements", "import", "instanceof", "interface", "native",
    "new", "package", "private", "protected", "public", "return", "static", "strictfp", "super", "switch",
    "synchronized", "this", "throw", "throws", "transient", "try", "volatile", "while", "var", "record", "fun", "val",
    "object", "when", "is", "namespace", "using", "internal", "sealed", "override", "readonly",
];
const JAVA_TY: &[&str] =
    &["int", "long", "short", "byte", "float", "double", "boolean", "char", "void", "String", "string", "bool"];

const SQL_KW: &[&str] = &[
    "select", "from", "where", "and", "or", "not", "insert", "into", "values", "update", "set", "delete", "create",
    "table", "alter", "drop", "index", "join", "inner", "left", "right", "outer", "full", "cross", "on", "as",
    "group", "by", "order", "having", "limit", "offset", "union", "all", "distinct", "in", "is", "like", "between",
    "exists", "case", "when", "then", "else", "end", "primary", "key", "foreign", "references", "default", "with",
    "view", "asc", "desc", "begin", "commit", "rollback", "if", "constraint", "unique", "check", "returning",
];
const SQL_TY: &[&str] = &[
    "int", "integer", "bigint", "smallint", "text", "varchar", "char", "boolean", "date", "timestamp", "float",
    "double", "decimal", "numeric", "serial", "uuid", "json", "jsonb", "blob",
];

macro_rules! lang {
    ($name:ident { $($f:ident : $v:expr),* $(,)? }) => {
        const $name: Spec = Spec { $($f: $v,)* ..PLAIN };
    };
}

lang!(RUST {
    line: &["//"], nested: true, quotes: "\"", rust_raw: true, keywords: RUST_KW, types: RUST_TY, consts: BOOLS,
    def_words: &["fn", "macro_rules"], at_ident: false,
});
lang!(JS {
    line: &["//"], quotes: "\"'`", keywords: JS_KW, types: JS_TY, consts: BOOLS, def_words: &["function"],
});
lang!(PY {
    line: &["#"], block: None, triple: true, quotes: "\"'", keywords: PY_KW, types: PY_TY, consts: BOOLS,
    def_words: &["def", "class"], at_ident: true,
});
lang!(JSON {
    line: &["//"], quotes: "\"", consts: BOOLS, keys: Keys::Json,
});
lang!(YAML {
    line: &["#"], block: None, quotes: "\"'", consts: &["true", "false", "null", "yes", "no", "on", "off", "~"],
    keys: Keys::Yaml,
});
lang!(TOML {
    line: &["#"], block: None, triple: true, quotes: "\"'", consts: BOOLS, keys: Keys::Toml,
});
lang!(SH {
    line: &["#"], block: None, quotes: "\"'`", keywords: SH_KW, dollar: true,
});
lang!(C {
    line: &["//"], quotes: "\"'", keywords: C_KW, types: C_TY, consts: &["true", "false", "NULL", "nullptr"],
    at_ident: false,
});
lang!(GO {
    line: &["//"], quotes: "\"'`", keywords: GO_KW, types: GO_TY, consts: &["true", "false", "nil", "iota"],
    def_words: &["func"],
});
lang!(JAVA {
    line: &["//"], quotes: "\"'", keywords: JAVA_KW, types: JAVA_TY, consts: &["true", "false", "null"],
    at_ident: true,
});
lang!(SQL {
    line: &["--"], quotes: "\"'`", keywords: SQL_KW, types: SQL_TY, consts: &["null", "true", "false"], ci: true,
});

pub fn spec(lang: &str) -> &'static Spec {
    match lang {
        "rust" | "rs" => &RUST,
        "js" | "javascript" | "jsx" | "mjs" | "cjs" | "ts" | "typescript" | "tsx" | "node" => &JS,
        "py" | "python" | "python3" => &PY,
        "json" | "jsonc" | "json5" | "canvas" => &JSON,
        "yaml" | "yml" => &YAML,
        "toml" | "ini" | "conf" => &TOML,
        "sh" | "bash" | "shell" | "zsh" | "console" | "fish" | "shellscript" => &SH,
        "c" | "h" | "cpp" | "c++" | "cc" | "hpp" | "cxx" | "objc" | "cs" | "csharp" => &C,
        "go" | "golang" => &GO,
        "java" | "kotlin" | "kt" | "scala" | "swift" | "dart" | "php" => &JAVA,
        "sql" | "mysql" | "postgres" | "postgresql" | "sqlite" => &SQL,
        _ => &PLAIN,
    }
}
