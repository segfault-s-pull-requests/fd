use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(feature = "completions")]
use anyhow::anyhow;
use clap::{
    builder::RangedU64ValueParser, value_parser, AppSettings, Arg, ArgAction, ArgEnum, ArgGroup,
    ArgMatches, Command, ErrorKind, Parser,
};
#[cfg(feature = "completions")]
use clap_complete::Shell;
use normpath::PathExt;

use crate::error::print_error;
use crate::exec::CommandSet;
use crate::filesystem;
#[cfg(unix)]
use crate::filter::OwnerFilter;
use crate::filter::SizeFilter;

// Type for options that don't have any values, but are used to negate
// earlier options
struct Negations;

impl clap::FromArgMatches for Negations {
    fn from_arg_matches(_: &ArgMatches) -> clap::Result<Self> {
        Ok(Negations)
    }

    fn update_from_arg_matches(&mut self, _: &ArgMatches) -> clap::Result<()> {
        Ok(())
    }
}

impl clap::Args for Negations {
    fn augment_args(cmd: Command<'_>) -> Command<'_> {
        Self::augment_args_for_update(cmd)
    }

    fn augment_args_for_update(cmd: Command<'_>) -> Command<'_> {
        cmd.arg(
            Arg::new("no-hidden")
                .long("no-hidden")
                .overrides_with("hidden")
                .hide(true)
                .long_help("Overrides --hidden."),
        )
        .arg(
            Arg::new("ignore")
                .long("ignore")
                .overrides_with("no-ignore")
                .hide(true)
                .long_help("Overrides --no-ignore."),
        )
        .arg(
            Arg::new("ignore-vcs")
                .long("ignore-vcs")
                .overrides_with("no-ignore-vcs")
                .hide(true)
                .long_help("Overrides --no-ignore-vcs."),
        )
        .arg(
            Arg::new("relative-path")
                .long("relative-path")
                .overrides_with("absolute-path")
                .hide(true)
                .long_help("Overrides --absolute-path."),
        )
        .arg(
            Arg::new("no-follow")
                .long("no-follow")
                .overrides_with("follow")
                .hide(true)
                .long_help("Overrides --follow."),
        )
    }
}

#[derive(Parser)]
#[clap(
    version,
    setting(AppSettings::DeriveDisplayOrder),
    dont_collapse_args_in_usage = true,
    after_help = "Note: `fd -h` prints a short and concise overview while `fd --help` gives all \
    details.",
    group(ArgGroup::new("execs").args(&["exec", "exec-batch", "list-details"]).conflicts_with_all(&[
            "max-results", "has-results", "count"])),
)]
pub struct Opts {
    /// Search hidden files and directories
    ///
    /// Include hidden directories and files in the search results (default:
    /// hidden files and directories are skipped). Files and directories are considered
    /// to be hidden if their name starts with a `.` sign (dot).
    /// The flag can be overriden with --no-hidden.
    #[clap(long, short = 'H', action, overrides_with = "hidden")]
    pub hidden: bool,
    /// Do not respect .(git|fd)ignore files
    ///
    /// Show search results from files and directories that would otherwise be
    ///   ignored by '.gitignore', '.ignore', '.fdignore', or the global ignore file.
    ///   The flag can be overridden with --ignore.
    #[clap(long, short = 'I', action, overrides_with = "no-ignore")]
    pub no_ignore: bool,
    /// Do not respect .gitignore files
    ///
    ///Show search results from files and directories that would otherwise be
    ///ignored by '.gitignore' files. The flag can be overridden with --ignore-vcs.
    #[clap(long, action, overrides_with = "no-ignore-vcs", hide_short_help = true)]
    pub no_ignore_vcs: bool,
    /// Do not respect .(git|fd)ignore files in parent directories
    ///
    /// Show search results from files and directories that would otherwise be
    /// ignored by '.gitignore', '.ignore', or '.fdignore' files in parent directories.
    #[clap(
        long,
        action,
        overrides_with = "no-ignore-parent",
        hide_short_help = true
    )]
    pub no_ignore_parent: bool,
    /// Do not respect the global ignore file
    #[clap(long, action, hide = true)]
    pub no_global_ignore_file: bool,
    /// Unrestricted search, alias for '--no-ignore --hidden'
    ///
    ///Perform an unrestricted search, including ignored and hidden files. This is
    ///an alias for '--no-ignore --hidden'.
    #[clap(long = "unrestricted", short = 'u', overrides_with_all(&["ignore", "no-hidden"]), action(ArgAction::Count), hide_short_help = true)]
    rg_alias_hidden_ignore: u8,
    /// Case-sensitive search (default: smart case)
    ///
    ///Perform a case-sensitive search. By default, fd uses case-insensitive
    ///searches, unless the pattern contains an uppercase character (smart case).
    #[clap(long, short = 's', action, overrides_with_all(&["ignore-case", "case-sensitive"]))]
    pub case_sensitive: bool,
    /// Case-insensitive search (default: smart case)
    ///
    /// Perform a case-insensitive search. By default, fd uses case-insensitive searches, unless
    /// the pattern contains an uppercase character (smart case).
    #[clap(long, short = 'i', action, overrides_with_all(&["case-sensitive", "ignore-case"]))]
    pub ignore_case: bool,
    /// Glob-based search (default: regular expression)
    ///
    /// Perform a glob-based search instead of a regular expression search.
    #[clap(
        long,
        short = 'g',
        action,
        conflicts_with("fixed-strings"),
        overrides_with("glob")
    )]
    pub glob: bool,
    /// Regular-expression based search (default)
    ///
    ///Perform a regular-expression based search (default). This can be used to override --glob.
    #[clap(long, action, overrides_with_all(&["glob", "regex"]), hide_short_help = true)]
    pub regex: bool,
    /// Treat pattern as literal string instead of regex
    ///
    /// Treat the pattern as a literal string instead of a regular expression. Note
    /// that this also performs substring comparison. If you want to match on an
    /// exact filename, consider using '--glob'.
    #[clap(
        long,
        short = 'F',
        alias = "literal",
        overrides_with("fixed-strings"),
        hide_short_help = true
    )]
    pub fixed_strings: bool,
    /// Show absolute instead of relative paths
    ///
    /// Shows the full path starting with the root as opposed to relative paths.
    /// The flag can be overridden with --relative-path.
    #[clap(long, short = 'a', action, overrides_with("absolute-path"))]
    pub absolute_path: bool,
    /// Use a long listing format with file metadata
    ///
    /// Use a detailed listing format like 'ls -l'. This is basically an alias
    /// for '--exec-batch ls -l' with some additional 'ls' options. This can be
    /// used to see more metadata, to show symlink targets and to achieve a
    /// deterministic sort order.
    #[clap(long, short = 'l', action, conflicts_with("absolute-path"))]
    pub list_details: bool,
    /// Follow symbolic links
    ///
    /// By default, fd does not descend into symlinked directories. Using this
    /// flag, symbolic links are also traversed.
    /// Flag can be overriden with --no-follow.
    #[clap(
        long,
        short = 'L',
        alias = "dereference",
        action,
        overrides_with("follow")
    )]
    pub follow: bool,
    /// Search full abs. path (default: filename only)
    ///
    /// By default, the search pattern is only matched against the filename (or
    /// directory name). Using this flag, the pattern is matched against the full
    /// (absolute) path. Example:
    ///     fd --glob -p '**/.git/config'
    #[clap(long, short = 'p', action, overrides_with("full-path"))]
    pub full_path: bool,
    /// Separate results by the null character
    ///
    /// Separate search results by the null character (instead of newlines).
    /// Useful for piping results to 'xargs'.
    #[clap(
        long = "print0",
        short = '0',
        action,
        overrides_with("print0"),
        conflicts_with("list-details"),
        hide_short_help = true
    )]
    pub null_separator: bool,
    /// Set maximum search depth (default: none)
    ///
    /// Limit the directory traversal to a given depth. By default, there is no
    /// limit on the search depth.
    #[clap(
        long,
        short = 'd',
        value_name = "depth",
        value_parser,
        alias("maxdepth")
    )]
    max_depth: Option<usize>,
    /// Only show results starting at given depth
    ///
    /// Only show search results starting at the given depth.
    /// See also: '--max-depth' and '--exact-depth'
    #[clap(long, value_name = "depth", hide_short_help = true, value_parser)]
    min_depth: Option<usize>,
    /// Only show results at exact given depth
    ///
    /// Only show search results at the exact given depth. This is an alias for
    /// '--min-depth <depth> --max-depth <depth>'.
    #[clap(long, value_name = "depth", hide_short_help = true, value_parser, conflicts_with_all(&["max-depth", "min-depth"]))]
    exact_depth: Option<usize>,
    /// Do not travers into matching directories
    ///
    /// Do not traverse into directories that match the search criteria. If
    /// you want to exclude specific directories, use the '--exclude=…' option.
    #[clap(long, hide_short_help = true, action, conflicts_with_all(&["size", "exact-depth"]))]
    pub prune: bool,
    /// Filter by type: file (f), directory (d), symlink (l),\nexecutable (x),
    /// empty (e), socket (s), pipe (p))
    ///
    /// Filter the search by type:
    ///
    ///   'f' or 'file':         regular files
    ///   'd' or 'directory':    directories
    ///   'l' or 'symlink':      symbolic links
    ///   's' or 'socket':       socket
    ///   'p' or 'pipe':         named pipe (FIFO)
    ///
    ///   'x' or 'executable':   executables
    ///   'e' or 'empty':        empty files or directories
    ///
    /// This option can be specified more than once to include multiple file types.
    /// Searching for '--type file --type symlink' will show both regular files as
    /// well as symlinks. Note that the 'executable' and 'empty' filters work differently:
    /// '--type executable' implies '--type file' by default. And '--type empty' searches
    /// for empty files and directories, unless either '--type file' or '--type directory'
    /// is specified in addition.
    ///
    /// Examples:
    ///
    ///   - Only search for files:
    ///       fd --type file …
    ///       fd -tf …
    ///   - Find both files and symlinks
    ///       fd --type file --type symlink …
    ///       fd -tf -tl …
    ///   - Find executable files:
    ///       fd --type executable
    ///       fd -tx
    ///   - Find empty files:
    ///       fd --type empty --type file
    ///       fd -te -tf
    ///   - Find empty directories:
    ///       fd --type empty --type directory
    ///       fd -te -td"
    #[clap(long = "type", short = 't', value_name = "filetype", hide_possible_values = true,
        arg_enum, action = ArgAction::Append, number_of_values = 1)]
    pub filetype: Option<Vec<FileType>>,
    /// Filter by file extension
    ///
    /// (Additionally) filter search results by their file extension. Multiple
    /// allowable file extensions can be specified.
    ///
    /// If you want to search for files without extension,
    /// you can use the regex '^[^.]+$' as a normal search pattern.
    #[clap(long = "extension", short = 'e', value_name = "ext", action = ArgAction::Append, number_of_values = 1)]
    pub extensions: Option<Vec<String>>,

    #[clap(flatten)]
    pub exec: Exec,

    /// Max number of arguments to run as a batch with -X
    ///
    /// Maximum number of arguments to pass to the command given with -X.
    /// If the number of results is greater than the given size,
    /// the command given with -X is run again with remaining arguments.
    /// A batch size of zero means there is no limit (default), but note
    /// that batching might still happen due to OS restrictions on the
    /// maximum length of command lines.
    #[clap(
        long,
        value_name = "size",
        hide_short_help = true,
        requires("exec-batch"),
        value_parser = value_parser!(usize),
        default_value_t
    )]
    pub batch_size: usize,
    /// Exclude entries that match the given glob pattern
    ///
    /// "Exclude files/directories that match the given glob pattern. This
    ///      overrides any other ignore logic. Multiple exclude patterns can be
    ///      specified.
    ///
    ///      Examples:
    ///        --exclude '*.pyc'
    ///        --exclude node_modules
    #[clap(long, short = 'E', value_name = "pattern", action = ArgAction::Append, number_of_values = 1)]
    pub exclude: Vec<String>,
    /// Add custom ignore-file in '.gitignore' format
    ///
    /// Add a custom ignore-file in '.gitignore' format. These files have a low
    /// precedence.
    #[clap(long, value_name = "path", action = ArgAction::Append, number_of_values = 1, hide_short_help = true)]
    pub ignore_file: Vec<PathBuf>,
    /// When to use colors
    #[clap(
        long,
        short = 'c',
        arg_enum,
        default_value = "auto",
        value_name = "when"
    )]
    pub color: ColorWhen,
    /// Set number of threads
    ///
    /// Set number of threads to use for searching & executing (default: number
    /// of available CPU cores)
    #[clap(long, short = 'j', value_name = "num", hide_short_help = true, value_parser = RangedU64ValueParser::<usize>::from(1..))]
    pub threads: Option<usize>,
    /// Limit results based on the size of files
    ///
    /// Limit results based on the size of files using the format <+-><NUM><UNIT>.
    ///     '+': file size must be greater than or equal to this
    ///     '-': file size must be less than or equal to this
    /// If neither '+' nor '-' is specified, file size must be exactly equal to this.
    ///     'NUM':  The numeric size (e.g. 500)
    ///     'UNIT': The units for NUM. They are not case-sensitive.
    /// Allowed unit values:
    ///     'b':  bytes
    ///     'k':  kilobytes (base ten, 10^3 = 1000 bytes)
    ///     'm':  megabytes
    ///     'g':  gigabytes
    ///     't':  terabytes
    ///     'ki': kibibytes (base two, 2^10 = 1024 bytes)
    ///     'mi': mebibytes
    ///     'gi': gibibytes
    ///     'ti': tebibytes
    #[clap(long, short = 'S', number_of_values = 1, value_parser = SizeFilter::from_string, allow_hyphen_values = true, action = ArgAction::Append)]
    pub size: Vec<SizeFilter>,
    /// Milliseconds to buffer before streaming search results to console
    ///
    /// Amount of time in milliseconds to buffer, before streaming the search
    /// results to the console.
    #[clap(long, hide = true, action, value_parser = parse_millis)]
    pub max_buffer_time: Option<Duration>,
    /// Filter by file modification time (newer than)
    ///
    /// Filter results based on the file modification time. The argument can be provided
    /// as a specific point in time (YYYY-MM-DD HH:MM:SS) or as a duration (10h, 1d, 35min).
    /// If the time is not specified, it defaults to 00:00:00.
    /// '--change-newer-than' or '--newer' can be used as aliases.
    /// Examples:
    ///     --changed-within 2weeks
    ///     --change-newer-than '2018-10-27 10:00:00'
    ///     --newer 2018-10-27
    #[clap(
        long,
        alias("change-newer-than"),
        alias("newer"),
        value_name = "date|dur",
        number_of_values = 1,
        action
    )]
    pub changed_within: Option<String>,
    /// Filter by file modification time (older than)
    ///
    /// Filter results based on the file modification time. The argument can be provided
    /// as a specific point in time (YYYY-MM-DD HH:MM:SS) or as a duration (10h, 1d, 35min).
    /// '--change-older-than' or '--older' can be used as aliases.
    ///
    /// Examples:
    ///     --changed-before '2018-10-27 10:00:00'
    ///     --change-older-than 2weeks
    ///     --older 2018-10-27
    #[clap(
        long,
        alias("change-older-than"),
        alias("older"),
        value_name = "date|dur",
        number_of_values = 1,
        action
    )]
    pub changed_before: Option<String>,
    /// Limit number of search results
    ///
    /// Limit the number of search results to 'count' and quit immediately.
    #[clap(long, value_name = "count", hide_short_help = true, value_parser)]
    max_results: Option<usize>,
    /// Limit search to a single result
    ///
    /// Limit the search to a single result and quit immediately.
    /// This is an alias for '--max-results=1'.
    #[clap(
        short = '1',
        hide_short_help = true,
        overrides_with("max-results"),
        action
    )]
    max_one_result: bool,
    /// Print nothing, exit code 0 if match found, 1 otherwise
    ///
    /// When the flag is present, the program does not print anything and will
    /// return with an exit code of 0 if there is at least one match. Otherwise, the
    /// exit code will be 1.
    ///
    /// '--has-results' can be used as an alias.
    #[clap(long, short = 'q', alias = "has-results", hide_short_help = true, conflicts_with("max-results"), action)]
    pub quiet: bool,
    /// Show filesystem errors
    ///
    ///Enable the display of filesystem errors for situations such as
    ///insufficient permissions or dead symlinks.
    #[clap(long, hide_short_help = true, overrides_with("show-errors"), action)]
    pub show_errors: bool,
    /// Change current working directory
    ///
    /// Change the current working directory of fd to the provided path. This
    /// means that search results will be shown with respect to the given base
    /// path. Note that relative paths which are passed to fd via the positional
    /// <path> argument or the '--search-path' option will also be resolved
    /// relative to this directory.
    #[clap(
        long,
        value_name = "path",
        number_of_values = 1,
        action,
        hide_short_help = true
    )]
    pub base_directory: Option<PathBuf>,
    /// the search pattern (a regular expression, unless '--glob' is used; optional)
    ///
    /// the search pattern which is either a regular expression (default) or a glob
    /// pattern (if --glob is used). If no pattern has been specified, every entry
    /// is considered a match. If your pattern starts with a dash (-), make sure to
    /// pass '--' first, or it will be considered as a flag (fd -- '-foo').
    #[clap(value_parser, default_value = "")]
    pub pattern: String,
    /// Set path separator when printing file paths
    /// Set the path separator to use when printing file paths. The default is
    /// the OS-specific separator ('/' on Unix, '\\' on Windows).
    #[clap(long, value_name = "separator", hide_short_help = true, action)]
    pub path_separator: Option<String>,
    /// the root directories for the filesystem search (optional)
    ///
    /// The directories where the filesystem search is rooted (optional).
    /// If omitted, search the current working directory.
    #[clap(action = ArgAction::Append)]
    path: Vec<PathBuf>,
    /// Provides paths to search as an alternative to the positional <path>
    ///
    /// Provide paths to search as an alternative to the positional <path>
    ///     argument. Changes the usage to `fd [OPTIONS] --search-path <path>
    ///     --search-path <path2> [<pattern>]`
    #[clap(long, conflicts_with("path"), action = ArgAction::Append, hide_short_help = true, number_of_values = 1)]
    search_path: Vec<PathBuf>,
    /// strip './' prefix from non-tty outputs
    ///
    /// By default, relative paths are prefixed with './' when the output goes to a non
    /// interactive terminal (TTY). Use this flag to disable this behaviour.
    #[clap(long, conflicts_with_all(&["path", "search-path"]), hide_short_help = true, action)]
    pub strip_cwd_prefix: bool,
    /// Filter by owning user and/or group
    ///
    /// Filter files by their user and/or group.
    /// Format: [(user|uid)][:(group|gid)]. Either side is optional.
    /// Precede either side with a '!' to exclude files instead.
    ///
    /// Examples:
    ///     --owner john
    ///     --owner :students
    ///     --owner '!john:students'
    #[cfg(unix)]
    #[clap(long, short = 'o', value_parser = OwnerFilter::from_string, value_name = "user:group")]
    pub owner: Option<OwnerFilter>,
    /// Do not descend into a different file system
    ///
    /// By default, fd will traverse the file system tree as far as other options
    /// dictate. With this flag, fd ensures that it does not descend into a
    /// different file system than the one it started in. Comparable to the -mount
    /// or -xdev filters of find(1).
    #[cfg(any(unix, windows))]
    #[clap(long, aliases(&["mount", "xdev"]), hide_short_help = true)]
    pub one_file_system: bool,

    #[cfg(feature = "completions")]
    #[clap(long, value_parser = value_parser!(Shell), hide = true, exclusive = true)]
    gen_completions: Option<Option<Shell>>,

    #[clap(flatten)]
    _negations: Negations,
}

impl Opts {
    pub fn search_paths(&self) -> anyhow::Result<Vec<PathBuf>> {
        // would it make sense to concatenate these?
        let paths = if !self.path.is_empty() {
            &self.path
        } else if !self.search_path.is_empty() {
            &self.search_path
        } else {
            let current_directory = Path::new(".");
            ensure_current_directory_exists(current_directory)?;
            return Ok(vec![self.normalize_path(current_directory)]);
        };
        Ok(paths
            .iter()
            .filter_map(|path| {
                if filesystem::is_existing_directory(&path) {
                    Some(self.normalize_path(path))
                } else {
                    print_error(format!(
                        "Search path '{}' is not a directory.",
                        path.to_string_lossy()
                    ));
                    None
                }
            })
            .collect())
    }

    fn normalize_path(&self, path: &Path) -> PathBuf {
        if self.absolute_path {
            filesystem::absolute_path(path.normalize().unwrap().as_path()).unwrap()
        } else {
            path.to_path_buf()
        }
    }

    pub fn no_search_paths(&self) -> bool {
        self.path.is_empty() && self.search_path.is_empty()
    }

    #[inline]
    pub fn rg_alias_ignore(&self) -> bool {
        self.rg_alias_hidden_ignore > 0
    }

    pub fn max_depth(&self) -> Option<usize> {
        self.max_depth.or(self.exact_depth)
    }

    pub fn min_depth(&self) -> Option<usize> {
        self.min_depth.or(self.exact_depth)
    }

    pub fn threads(&self) -> usize {
        std::cmp::max(self.threads.unwrap_or_else(num_cpus::get), 1)
    }

    pub fn max_results(&self) -> Option<usize> {
        self.max_results.filter(|&m| m > 0).or_else(|| self.max_one_result.then(|| 1))
    }

    #[cfg(feature = "completions")]
    pub fn gen_completions(&self) -> anyhow::Result<Option<Shell>> {
        self.gen_completions
            .map(|maybe_shell| match maybe_shell {
                Some(sh) => Ok(sh),
                None => guess_shell(),
            })
            .transpose()
    }
}

// TODO: windows?
#[cfg(feature = "completions")]
fn guess_shell() -> anyhow::Result<Shell> {
    let env_shell = std::env::var_os("SHELL").map(PathBuf::from);
    let shell = env_shell.as_ref()
        .and_then(|s| s.file_name())
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("Unable to get shell from environment"))?;
    shell
        .parse::<Shell>()
        .map_err(|_| anyhow!("Unknown shell {}", shell))
}

#[derive(Copy, Clone, PartialEq, Eq, ArgEnum)]
pub enum FileType {
    #[clap(alias = "f")]
    File,
    #[clap(alias = "d")]
    Directory,
    #[clap(alias = "l")]
    Symlink,
    #[clap(alias = "x")]
    Executable,
    #[clap(alias = "e")]
    Empty,
    #[clap(alias = "s")]
    Socket,
    #[clap(alias = "p")]
    Pipe,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ArgEnum)]
pub enum ColorWhen {
    /// show colors if the output goes to an interactive console (default)
    Auto,
    /// always use colorized output
    Always,
    /// do not use colorized output
    Never,
}

// there isn't a derive api for getting grouped values yet,
// so we have to use hand-rolled parsing for exec and exec-batch
pub struct Exec {
    pub command: Option<CommandSet>,
}

impl clap::FromArgMatches for Exec {
    fn from_arg_matches(matches: &ArgMatches) -> clap::Result<Self> {
        let command = matches
            .grouped_values_of("exec")
            .map(CommandSet::new)
            .or_else(|| {
                matches
                    .grouped_values_of("exec-batch")
                    .map(CommandSet::new_batch)
            })
            .transpose()
            .map_err(|e| clap::Error::raw(ErrorKind::InvalidValue, e))?;
        Ok(Exec { command })
    }

    fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> clap::Result<()> {
        *self = Self::from_arg_matches(matches)?;
        Ok(())
    }
}

impl clap::Args for Exec {
    fn augment_args(cmd: Command<'_>) -> Command<'_> {
        cmd.arg(Arg::new("exec")
            .long("exec")
            .short('x')
            .min_values(1)
                .multiple_occurrences(true)
                .allow_hyphen_values(true)
                .value_terminator(";")
                .value_name("cmd")
                .conflicts_with("list-details")
                .help("Execute a command for each search result")
                .long_help(
                    "Execute a command for each search result in parallel (use --threads=1 for sequential command execution). \
                     All positional arguments following --exec are considered to be arguments to the command - not to fd. \
                     It is therefore recommended to place the '-x'/'--exec' option last.\n\
                     The following placeholders are substituted before the command is executed:\n  \
                       '{}':   path (of the current search result)\n  \
                       '{/}':  basename\n  \
                       '{//}': parent directory\n  \
                       '{.}':  path without file extension\n  \
                       '{/.}': basename without file extension\n\n\
                     If no placeholder is present, an implicit \"{}\" at the end is assumed.\n\n\
                     Examples:\n\n  \
                       - find all *.zip files and unzip them:\n\n      \
                           fd -e zip -x unzip\n\n  \
                       - find *.h and *.cpp files and run \"clang-format -i ..\" for each of them:\n\n      \
                           fd -e h -e cpp -x clang-format -i\n\n  \
                       - Convert all *.jpg files to *.png files:\n\n      \
                           fd -e jpg -x convert {} {.}.png\
                    ",
                ),
        )
        .arg(
            Arg::new("exec-batch")
                .long("exec-batch")
                .short('X')
                .min_values(1)
                .multiple_occurrences(true)
                .allow_hyphen_values(true)
                .value_terminator(";")
                .value_name("cmd")
                .conflicts_with_all(&["exec", "list-details"])
                .help("Execute a command with all search results at once")
                .long_help(
                    "Execute the given command once, with all search results as arguments.\n\
                     One of the following placeholders is substituted before the command is executed:\n  \
                       '{}':   path (of all search results)\n  \
                       '{/}':  basename\n  \
                       '{//}': parent directory\n  \
                       '{.}':  path without file extension\n  \
                       '{/.}': basename without file extension\n\n\
                     If no placeholder is present, an implicit \"{}\" at the end is assumed.\n\n\
                     Examples:\n\n  \
                       - Find all test_*.py files and open them in your favorite editor:\n\n      \
                           fd -g 'test_*.py' -X vim\n\n  \
                       - Find all *.rs files and count the lines with \"wc -l ...\":\n\n      \
                           fd -e rs -X wc -l\
                     "
                ),
        )
    }

    fn augment_args_for_update(cmd: Command<'_>) -> Command<'_> {
        Self::augment_args(cmd)
    }
}

fn parse_millis(arg: &str) -> Result<Duration, std::num::ParseIntError> {
    Ok(Duration::from_millis(arg.parse()?))
}

fn ensure_current_directory_exists(current_directory: &Path) -> anyhow::Result<()> {
    if filesystem::is_existing_directory(current_directory) {
        Ok(())
    } else {
        Err(anyhow!(
            "Could not retrieve current directory (has it been deleted?)."
        ))
    }
}

