mod html;
mod markdown;
mod paragraph;

use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::path::PathBuf;
use std::process;

use anyhow::{anyhow, Context, Error};
use markdown::DocumentSource;
use rayon::prelude::*;
use structopt::StructOpt;
use walkdir::WalkDir;

use html::{Document, Href, Link};

#[derive(StructOpt)]
#[structopt(name = "hyperlink")]
struct Cli {
    /// The static file path to check.
    ///
    /// This will be assumed to be the root path of your server as well, so
    /// href="/foo" will resolve to that folder's subfolder foo.
    #[structopt(verbatim_doc_comment)]
    base_path: PathBuf,

    /// How many threads to use, default is to try and saturate CPU.
    #[structopt(short = "j", long = "jobs")]
    threads: Option<usize>,

    /// Whether to check for valid anchor references.
    #[structopt(long = "check-anchors")]
    check_anchors: bool,

    /// Path to directory of markdown files to use for reporting errors.
    #[structopt(long = "sources")]
    sources_path: Option<PathBuf>,

    /// Enable specialized output for GitHub actions.
    #[structopt(long = "github-actions")]
    github_actions: bool,
}

fn main() -> Result<(), Error> {
    let Cli {
        base_path,
        threads,
        check_anchors,
        sources_path,
        github_actions,
    } = Cli::from_args();

    if let Some(n) = threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .unwrap();
    }

    let mut defined_links = BTreeSet::new();
    let mut documents = Vec::new();

    println!("Discovering files");

    for entry in WalkDir::new(&base_path) {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let file_type = metadata.file_type();

        if file_type.is_symlink() {
            return Err(anyhow!(
                "Found unsupported symlink at {}",
                entry.path().display()
            ));
        }

        if !file_type.is_file() {
            continue;
        }

        let document = Document::new(&base_path, entry.into_path());

        if !defined_links.insert(document.href.clone()) {
            panic!("Found two files that would probably serve the same href. One of them is {}. Please file a bug with the output of 'find' on your folder.", document.path.display());
        }

        if document
            .path
            .extension()
            .map_or(false, |extension| extension == "html" || extension == "htm")
        {
            documents.push(document);
        }
    }

    println!(
        "Checking {} out of {} files",
        documents.len(),
        defined_links.len()
    );

    let links_result: Result<_, Error> = documents
        .par_iter()
        .map_init(Vec::new, |buf, document| {
            let mut links = Vec::new();

            document
                .links(buf, check_anchors, sources_path.is_some(), |link| {
                    links.push(link)
                })
                .with_context(|| format!("Failed to read file {}", document.path.display()))?;

            Ok(links)
        })
        .try_reduce(Vec::new, |mut links, links2| {
            links.extend(links2);
            Ok(links)
        });

    let mut used_links = BTreeMap::new();

    for link in links_result? {
        match link {
            Link::Uses(used_link) => {
                used_links
                    .entry(used_link.href.clone())
                    .or_insert_with(Vec::new)
                    .push(used_link);
            }
            Link::Defines(defined_link) => {
                // XXX: Use whole link
                defined_links.insert(defined_link.href);
            }
        }
    }

    let mut paragraps_to_sourcefile = BTreeMap::new();

    if let Some(ref sources_path) = sources_path {
        println!("Discovering source files");

        let mut file_count = 0;
        let mut document_sources = Vec::new();

        for entry in WalkDir::new(sources_path) {
            file_count += 1;
            let entry = entry?;
            let metadata = entry.metadata()?;
            let file_type = metadata.file_type();

            if !file_type.is_file() {
                continue;
            }

            let source = DocumentSource::new(entry.into_path());

            if source
                .path
                .extension()
                .map_or(false, |extension| extension == "mdx" || extension == "md")
            {
                document_sources.push(source);
            }
        }

        println!(
            "Checking {} out of {} files in source folder",
            document_sources.len(),
            file_count
        );

        let results: Vec<_> = document_sources
            .par_iter()
            .map(|source| -> Result<_, Error> {
                // XXX: Inefficient
                let mut paragraphs = Vec::new();
                source
                    .paragraphs(|p| paragraphs.push(p))
                    .with_context(|| format!("Failed to read file {}", source.path.display()))?;
                Ok((source, paragraphs))
            })
            .collect();

        for result in results {
            let (source, paragraphs) = result?;
            for paragraph in paragraphs {
                paragraps_to_sourcefile
                    .entry(paragraph)
                    .or_insert_with(Vec::new)
                    .push(source.clone());
            }
        }
    }

    let used_links_len = used_links.len();
    let mut bad_links_and_anchors = BTreeMap::new();
    let mut bad_links_count = 0;
    let mut bad_anchors_count = 0;

    for (href, links) in used_links {
        if !defined_links.contains(&href) {
            let hard_404 = !check_anchors || !defined_links.contains(&href.without_anchor());
            if hard_404 {
                bad_links_count += 1;
            } else {
                bad_anchors_count += 1;
            }

            for link in links {
                let mut had_sources = false;

                if let Some(ref paragraph) = link.paragraph {
                    if let Some(document_sources) = &paragraps_to_sourcefile.get(paragraph) {
                        debug_assert!(!document_sources.is_empty());
                        had_sources = true;

                        for source in *document_sources {
                            let (bad_links, bad_anchors) = bad_links_and_anchors
                                .entry((!had_sources, source.path.as_path()))
                                .or_insert_with(|| (Vec::new(), Vec::new()));

                            if hard_404 { bad_links } else { bad_anchors }.push(href.clone());
                        }
                    }
                }

                if !had_sources {
                    let (bad_links, bad_anchors) = bad_links_and_anchors
                        .entry((!had_sources, link.path))
                        .or_insert_with(|| (Vec::new(), Vec::new()));

                    if hard_404 { bad_links } else { bad_anchors }.push(href.clone());
                }
            }
        }
    }

    // _is_raw_file is an unused parameter that is only there to control iteration order over keys.
    // Sort markdown files to the start since otherwise the less valuable annotations on not
    // checked in files fill up the limit on annotations (tested manually, seems to be 10 right
    // now).
    for ((_is_raw_file, filepath), (bad_links, bad_anchors)) in bad_links_and_anchors {
        println!("{}", filepath.display());

        for href in &bad_links {
            println!("  error: bad link {}", href);
        }

        for href in &bad_anchors {
            println!("  warning: bad anchor {}", href);
        }

        if github_actions {
            if !bad_links.is_empty() {
                print!(
                    "::error file={}::bad links:",
                    filepath.canonicalize()?.display()
                );
                print_github_actions_href_list(&bad_links);
                println!();
            }

            if !bad_anchors.is_empty() {
                print!(
                    "::error file={}::bad anchors:",
                    filepath.canonicalize()?.display()
                );

                print_github_actions_href_list(&bad_anchors);
                println!();
            }
        }

        println!();
    }

    println!("Checked {} links", used_links_len);
    println!("Checked {} files", documents.len());
    println!("Found {} bad links", bad_links_count);

    if check_anchors {
        println!("Found {} bad anchors", bad_anchors_count);
    }

    // We're about to exit the program and leaking the memory is faster than running drop
    mem::forget(defined_links);
    mem::forget(documents);

    if bad_links_count > 0 {
        process::exit(1);
    }

    if bad_anchors_count > 0 {
        process::exit(2);
    }

    Ok(())
}

fn print_github_actions_href_list(hrefs: &[Href]) {
    for href in hrefs {
        // %0A -- escaped newline
        //
        // https://github.community/t/what-is-the-correct-character-escaping-for-workflow-command-values-e-g-echo-xxxx/118465/5
        print!("%0A  {}", href);
    }
}
