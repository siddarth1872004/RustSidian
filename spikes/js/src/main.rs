use boa_engine::{Context, Source};
use std::time::Instant;

const PLUGIN: &str = r#"
var module = { exports: {} };
(function (module, exports, require) {
  class Plugin { constructor(app){ this.app = app; this.cmds = []; } addCommand(c){ this.cmds.push(c); } }
  class MyPlugin extends Plugin {
    async onload() {
      for (let i = 0; i < 200; i++) this.addCommand({ id: 'cmd-' + i, name: `Command ${i}`, callback: () => i * 2 });
      const m = new Map(); for (const c of this.cmds) m.set(c.id, c);
      this.sum = [...m.values()].map(c => c.callback()).reduce((a, b) => a + b, 0);
      this.re = /(\[\[)([^\]]+)(\]\])/g.exec('see [[Note A]] now')[2];
    }
  }
  module.exports = MyPlugin;
})(module, module.exports, function(){ return {}; });
var p = new module.exports({}); p.onload(); JSON.stringify({ n: p.cmds.length, sum: p.sum, re: p.re });
"#;

const HOT: &str = r#"
function fib(n){ return n < 2 ? n : fib(n-1) + fib(n-2); }
let s = 0; const a = [];
for (let i = 0; i < 200000; i++) { a.push({ k: 'x' + (i % 97), v: i }); }
for (const o of a) { s += o.v % 7; }
fib(24) + s;
"#;

fn main() {
    let t = Instant::now();
    let mut ctx = Context::default();
    let init = t.elapsed();
    let t = Instant::now();
    let r = ctx.eval(Source::from_bytes(PLUGIN)).unwrap();
    let plug = t.elapsed();
    println!("context init {init:?}; plugin-shaped bundle eval {plug:?} -> {}", r.display());
    let t = Instant::now();
    let r = ctx.eval(Source::from_bytes(HOT)).unwrap();
    println!("hot loop (200k objs + fib(24)) {:?} -> {}", t.elapsed(), r.display());
}
