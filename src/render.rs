use std::collections::BTreeMap;
use std::path::Path;

use crate::load::{self, Loaded};
use crate::parse::{self, Seg};
use crate::sink;

pub fn fnv1a32(data: &[u8]) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for &b in data {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{:08x}", h)
}

#[derive(Clone, Copy, PartialEq)]
pub enum Status {
    New,
    Fresh,
    Stale,
    Rewritten,
    Edited,
    Error,
}

impl Status {
    pub fn label(&self) -> &'static str {
        match self {
            Status::New => "new",
            Status::Fresh => "fresh",
            Status::Stale => "stale",
            Status::Rewritten => "rewritten",
            Status::Edited => "edited",
            Status::Error => "error",
        }
    }
}

pub struct RegionReport {
    pub name: String,
    pub loader: String,
    pub status: Status,
    pub message: String,
    pub in_sum: Option<String>,
    pub out_sum: Option<String>,
}

pub struct FileReport {
    pub regions: Vec<RegionReport>,
    pub prose_drift: bool,
}

pub fn banner_for(tmpl_name: &str) -> String {
    format!("<!-- generated from {} by computed; edit the template, not this file -->", tmpl_name)
}

pub fn prior_regions(output: &str) -> BTreeMap<String, parse::Region> {
    let mut m = BTreeMap::new();
    for seg in parse::parse(output) {
        if let Seg::Region(r) = seg {
            m.insert(r.name.clone(), r);
        }
    }
    m
}

pub fn prose_of(text: &str, banner: &str) -> String {
    let mut parts = Vec::new();
    for seg in parse::parse(text) {
        if let Seg::Prose(p) = seg {
            parts.push(p);
        }
    }
    parts.join("\n").replace(banner, "").trim().to_string()
}

pub fn run_exit(rep: &FileReport) -> i32 {
    if rep.regions.iter().any(|r| matches!(r.status, Status::Error | Status::Edited)) {
        1
    } else {
        0
    }
}

pub fn check_exit(rep: &FileReport) -> i32 {
    if rep.prose_drift || rep.regions.iter().any(|r| r.status != Status::Fresh) {
        1
    } else {
        0
    }
}

pub fn render_file(
    root: &Path,
    tmpl: &Path,
    out: &Path,
    prior_text: Option<&str>,
    force: bool,
    trusted: bool,
) -> (String, FileReport) {
    let tmpl_text = std::fs::read_to_string(tmpl).unwrap_or_default();
    let tmpl_name = tmpl
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let banner = banner_for(&tmpl_name);
    let prior = prior_text.map(prior_regions).unwrap_or_default();
    let out_rel = out
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let exclude = vec![out_rel, ".computed-trust".to_string()];

    let mut lines: Vec<String> = vec![banner.clone()];
    let mut regions = Vec::new();

    for seg in parse::parse(&tmpl_text) {
        match seg {
            Seg::Prose(p) => lines.push(p),
            Seg::Error { line, message } => {
                lines.push(line);
                regions.push(RegionReport {
                    name: "(parse)".to_string(),
                    loader: String::new(),
                    status: Status::Error,
                    message,
                    in_sum: None,
                    out_sum: None,
                });
            }
            Seg::Region(r) => {
                let p = prior.get(&r.name);
                let edited = p
                    .map(|pr| {
                        pr.out_sum
                            .as_deref()
                            .is_some_and(|os| fnv1a32(pr.body.as_bytes()) != os)
                    })
                    .unwrap_or(false);

                let loaded: Result<Loaded, String> = match r.loader.as_str() {
                    "tree" => load::tree(root, &r.attrs, &exclude),
                    "csv" => load::csv(root, &r.attrs),
                    "sh" => load::sh(root, &r.attrs, trusted),
                    other => Err(format!("unknown loader \"{}\"", other)),
                };

                let computed = loaded.and_then(|l| {
                    let sink_name = r
                        .attrs
                        .get("sink")
                        .cloned()
                        .unwrap_or_else(|| load::default_sink(&r.loader).to_string());
                    sink::render(&sink_name, &l).map(|body| {
                        let in_sum =
                            fnv1a32(format!("{}\n{}", r.open_line, l.snapshot.unwrap_or_default()).as_bytes());
                        (body, in_sum)
                    })
                });

                let body: String;
                let in_sum: Option<String>;
                let out_sum: Option<String>;
                let status: Status;
                let message: String;
                match computed {
                    Err(e) => {
                        body = p.map(|pr| pr.body.clone()).unwrap_or_else(|| "(no previous output)".to_string());
                        in_sum = p.and_then(|pr| pr.in_sum.clone());
                        out_sum = p.and_then(|pr| pr.out_sum.clone());
                        status = Status::Error;
                        message = format!("{}. Last good content kept.", e);
                    }
                    Ok((cbody, cin)) => {
                        if edited && !force {
                            body = p.map(|pr| pr.body.clone()).unwrap_or_default();
                            in_sum = p.and_then(|pr| pr.in_sum.clone());
                            out_sum = p.and_then(|pr| pr.out_sum.clone());
                            status = Status::Edited;
                            message = "region was edited by hand; left as is. Delete the sum or run with force to regenerate.".to_string();
                        } else {
                            body = cbody.clone();
                            in_sum = Some(cin.clone());
                            out_sum = Some(fnv1a32(cbody.as_bytes()));
                            let same_inputs = p.and_then(|pr| pr.in_sum.as_deref()) == Some(cin.as_str());
                            let same_body = p.map(|pr| pr.body == cbody).unwrap_or(false);
                            let mut msg = if same_inputs && same_body {
                                status = Status::Fresh;
                                "inputs unchanged"
                            } else if same_inputs {
                                status = Status::Rewritten;
                                "same inputs, different content (undeclared inputs)"
                            } else if p.is_some() {
                                status = Status::Stale;
                                "inputs changed; regenerated"
                            } else {
                                status = Status::New;
                                "first render"
                            };
                            if edited && force {
                                msg = "hand edit discarded (force)";
                            }
                            message = msg.to_string();
                        }
                    }
                }

                lines.push(format!("{}{}", r.indent, r.open_line));
                lines.push(body.clone());
                let mut closer = format!("{}<!-- /computed", r.indent);
                if let Some(s) = &in_sum {
                    closer.push_str(&format!(" sum={}", s));
                }
                if let Some(s) = &out_sum {
                    closer.push_str(&format!(" out={}", s));
                }
                closer.push_str(" -->");
                lines.push(closer);

                regions.push(RegionReport {
                    name: r.name.clone(),
                    loader: r.loader.clone(),
                    status,
                    message,
                    in_sum,
                    out_sum,
                });
            }
        }
    }

    let text = lines.join("\n");
    let prose_drift = match prior_text {
        Some(p) => prose_of(p, &banner) != prose_of(&tmpl_text, &banner),
        None => false,
    };
    (text, FileReport { regions, prose_drift })
}
