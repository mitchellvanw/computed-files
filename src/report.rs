use std::fs;
use std::path::Path;

use crate::render::FileReport;

fn pad(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len >= n {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(n - len))
    }
}

pub fn print_report(kind: &str, exit: i32, wrote: Option<bool>, rep: &FileReport) {
    let mut head = format!("{} → exit {}", kind, exit);
    if let Some(w) = wrote {
        head.push_str(if w { " · wrote" } else { " · write skipped (unchanged)" });
    }
    println!("{}", head);
    for r in &rep.regions {
        let sums = match (&r.in_sum, &r.out_sum) {
            (Some(i), Some(o)) => format!("  sum={} out={}", i, o),
            (Some(i), None) => format!("  sum={}", i),
            _ => String::new(),
        };
        println!(
            "  {} {} {} {}{}",
            pad(&r.name, 10),
            pad(&r.loader, 5),
            pad(r.status.label(), 10),
            r.message,
            sums
        );
    }
    if rep.prose_drift {
        println!(
            "  {} {} {} prose outside regions differs from the template; the template wins",
            pad("prose", 10),
            pad("", 5),
            pad("drift", 10)
        );
    }
}

pub fn print_output(path: &Path) {
    match fs::read_to_string(path) {
        Ok(text) => {
            println!("\n── {} · {} lines ──", path.display(), text.lines().count());
            print!("{}", text);
        }
        Err(_) => println!("\n── {} does not exist ──", path.display()),
    }
}
