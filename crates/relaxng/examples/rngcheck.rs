//! `rngcheck SCHEMA.rng FILE…`: print `FILE<TAB>valid|invalid|error` for
//! each file (the format of `tools/oracle/probes/relaxng/verdicts.pl`), and
//! the validity errors on stderr.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((schema, files)) = args.split_first() else {
        eprintln!("usage: rngcheck SCHEMA.rng FILE...");
        std::process::exit(2);
    };
    let schema = match relaxng::Schema::from_file(std::path::Path::new(schema)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{schema}: {e}");
            std::process::exit(2);
        }
    };
    let opts = relaxng::Options {
        libxml2_interleave: std::env::var_os("RNG_LIBXML2").is_some(),
    };
    let mut files = files.to_vec();
    if files == ["-"] {
        files = std::io::stdin().lines().map_while(Result::ok).collect();
    }
    for f in &files {
        let text = match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(e) => {
                println!("{f}\terror");
                eprintln!("{f}: {e}");
                continue;
            }
        };
        match schema.validate_str_with(&text, &opts) {
            Ok(()) => println!("{f}\tvalid"),
            Err(relaxng::ValidateError::NotWellFormed(e)) => {
                println!("{f}\terror");
                eprintln!("{f}: {e}");
            }
            Err(relaxng::ValidateError::Invalid(errs)) => {
                println!("{f}\tinvalid");
                for e in errs {
                    eprintln!("{}", e.display_in(f));
                }
            }
        }
    }
}
