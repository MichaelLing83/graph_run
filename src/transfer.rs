//! SFTP transfers between [[servers]] (local or remote), preserving mode and timestamps like `rsync -a`.

use std::fs;
use std::io;
use std::path::Path;

use ssh2::{FileStat, OpenFlags, OpenType, Session, Sftp};
use std::net::TcpStream;
use std::time::Duration;

use crate::config::{ConfigBundle, Server, TransferSpec};
use crate::error::{GraphRunError, Result};

const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const S_IFLNK: u32 = 0o120_000;

fn perm_kind(perm: Option<u32>) -> Option<u32> {
    perm.map(|p| p & S_IFMT)
}

fn is_dir(st: &FileStat) -> bool {
    perm_kind(st.perm) == Some(S_IFDIR)
}

fn is_reg(st: &FileStat) -> bool {
    perm_kind(st.perm) == Some(S_IFREG)
}

fn is_lnk(st: &FileStat) -> bool {
    perm_kind(st.perm) == Some(S_IFLNK)
}

fn mode_for_file(st: &FileStat) -> i32 {
    (st.perm.unwrap_or(0o644) & 0o7777) as i32
}

fn mode_for_mkdir(st: &FileStat) -> i32 {
    (st.perm.unwrap_or(0o755) & 0o7777) as i32
}

fn ssh_err(e: ssh2::Error) -> GraphRunError {
    GraphRunError::msg(format!("SSH/SFTP: {e}"))
}

fn connect_session(server: &Server, timeout_ms: u32) -> Result<Session> {
    if server.kind != "remote" {
        return Err(GraphRunError::msg(format!(
            "transfer: server {} is not remote (kind={})",
            server.id, server.kind
        )));
    }
    let host = server
        .host
        .as_deref()
        .ok_or_else(|| GraphRunError::msg(format!("transfer: remote server {} has no host", server.id)))?;
    let user = server
        .user
        .as_deref()
        .ok_or_else(|| GraphRunError::msg(format!("transfer: remote server {} has no user", server.id)))?;
    let port = server.port.unwrap_or(22);
    let addr = format!("{host}:{port}");
    let tcp = TcpStream::connect(&addr).map_err(|e| GraphRunError::msg(format!("SSH connect {addr}: {e}")))?;
    let _ = tcp.set_read_timeout(Some(Duration::from_millis(timeout_ms as u64)));
    let _ = tcp.set_write_timeout(Some(Duration::from_millis(timeout_ms as u64)));

    let mut sess = Session::new().map_err(ssh_err)?;
    sess.set_tcp_stream(tcp);
    sess.set_blocking(true);
    sess.set_timeout(timeout_ms);
    sess.handshake().map_err(ssh_err)?;

    if let Some(pw) = server.resolved_password() {
        if !pw.is_empty() {
            sess.userauth_password(user, &pw).map_err(ssh_err)?;
        }
    }
    if !sess.authenticated() {
        sess.userauth_agent(user).map_err(ssh_err)?;
    }
    if !sess.authenticated() {
        return Err(GraphRunError::msg(format!(
            "SSH authentication failed for server {} ({}@{})",
            server.id, user, host
        )));
    }
    Ok(sess)
}

fn mkdir_p_remote(sftp: &Sftp, path: &Path, default_mode: i32) -> Result<()> {
    if path.as_os_str().is_empty() || path == Path::new("/") {
        return Ok(());
    }
    match sftp.stat(path) {
        Ok(st) => {
            if is_dir(&st) {
                return Ok(());
            }
            return Err(GraphRunError::msg(format!(
                "mkdir_p: {} exists and is not a directory",
                path.display()
            )));
        }
        Err(_) => {}
    }
    if let Some(parent) = path.parent() {
        mkdir_p_remote(sftp, parent, default_mode)?;
    }
    sftp.mkdir(path, default_mode).map_err(ssh_err)?;
    Ok(())
}

fn mkdir_p_local(path: &Path, default_mode: u32) -> Result<()> {
    fs::create_dir_all(path).map_err(|e| GraphRunError::msg(format!("mkdir {}: {e}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::Permissions::from_mode(default_mode & 0o777);
        let _ = fs::set_permissions(path, mode);
    }
    Ok(())
}

fn setstat_remote(sftp: &Sftp, path: &Path, st: &FileStat) -> Result<()> {
    sftp
        .setstat(
            path,
            FileStat {
                size: None,
                uid: st.uid,
                gid: st.gid,
                perm: st.perm,
                atime: st.atime,
                mtime: st.mtime,
            },
        )
        .map_err(ssh_err)
}

fn copy_remote_file_to_remote(sa: &Sftp, sb: &Sftp, from: &Path, to: &Path, st: &FileStat) -> Result<()> {
    if let Some(parent) = to.parent() {
        mkdir_p_remote(sb, parent, 0o755)?;
    }
    let mut rf = sa
        .open_mode(from, OpenFlags::READ, 0, OpenType::File)
        .map_err(ssh_err)?;
    let mode = mode_for_file(st);
    let mut wf = sb
        .open_mode(
            to,
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE,
            mode,
            OpenType::File,
        )
        .map_err(ssh_err)?;
    io::copy(&mut rf, &mut wf).map_err(|e| GraphRunError::msg(format!("sftp copy: {e}")))?;
    wf.setstat(FileStat {
        size: st.size,
        uid: st.uid,
        gid: st.gid,
        perm: st.perm,
        atime: st.atime,
        mtime: st.mtime,
    })
    .map_err(ssh_err)?;
    Ok(())
}

fn copy_remote_symlink_to_remote(sa: &Sftp, sb: &Sftp, from: &Path, to: &Path) -> Result<()> {
    let target = sa.readlink(from).map_err(ssh_err)?;
    if let Some(parent) = to.parent() {
        mkdir_p_remote(sb, parent, 0o755)?;
    }
    sb.symlink(&target, to).map_err(ssh_err)
}

fn copy_remote_tree_inner(
    sa: &Sftp,
    sb: &Sftp,
    src: &Path,
    dst: &Path,
    src_contents_only: bool,
) -> Result<()> {
    let st = sa.lstat(src).map_err(ssh_err)?;
    if is_lnk(&st) {
        return copy_remote_symlink_to_remote(sa, sb, src, dst);
    }
    if is_reg(&st) {
        return copy_remote_file_to_remote(sa, sb, src, dst, &st);
    }
    if !is_dir(&st) {
        return Err(GraphRunError::msg(format!(
            "unsupported source file type (not file/dir/symlink): {}",
            src.display()
        )));
    }

    if src_contents_only {
        mkdir_p_remote(sb, dst, mode_for_mkdir(&st))?;
        setstat_remote(sb, dst, &st)?;
        for (name, _) in sa.readdir(src).map_err(ssh_err)? {
            if name == Path::new(".") || name == Path::new("..") {
                continue;
            }
            copy_remote_tree_inner(sa, sb, &src.join(&name), &dst.join(&name), false)?;
        }
        return Ok(());
    }

    mkdir_p_remote(sb, dst, mode_for_mkdir(&st))?;
    setstat_remote(sb, dst, &st)?;
    for (name, _) in sa.readdir(src).map_err(ssh_err)? {
        if name == Path::new(".") || name == Path::new("..") {
            continue;
        }
        copy_remote_tree_inner(sa, sb, &src.join(&name), &dst.join(&name), false)?;
    }
    Ok(())
}

fn copy_remote_to_remote(
    src_srv: &Server,
    dst_srv: &Server,
    spec: &TransferSpec,
    timeout_ms: u32,
) -> Result<()> {
    let sa = connect_session(src_srv, timeout_ms)?;
    let sb = connect_session(dst_srv, timeout_ms)?;
    let sftp_a = sa.sftp().map_err(ssh_err)?;
    let sftp_b = sb.sftp().map_err(ssh_err)?;
    let src = Path::new(&spec.source_path);
    let dst = Path::new(&spec.dest_path);
    let src_slash = spec.source_path.ends_with('/');
    copy_remote_tree_inner(&sftp_a, &sftp_b, src, dst, src_slash)
}

fn copy_local_file_to_remote(sftp: &Sftp, from: &Path, to: &Path, meta: &fs::Metadata) -> Result<()> {
    if let Some(parent) = to.parent() {
        mkdir_p_remote(sftp, parent, 0o755)?;
    }
    let mut rf = fs::File::open(from).map_err(|e| GraphRunError::msg(format!("open {}: {e}", from.display())))?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        (meta.permissions().mode() & 0o7777) as i32
    };
    #[cfg(not(unix))]
    let mode = 0o644;
    let mut wf = sftp
        .open_mode(
            to,
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE,
            mode,
            OpenType::File,
        )
        .map_err(ssh_err)?;
    io::copy(&mut rf, &mut wf).map_err(|e| GraphRunError::msg(format!("copy: {e}")))?;
    let atime = meta.accessed().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    wf.setstat(FileStat {
        size: Some(meta.len()),
        uid: None,
        gid: None,
        perm: None,
        atime,
        mtime,
    })
    .map_err(ssh_err)?;
    Ok(())
}

fn copy_local_tree_to_remote(sftp: &Sftp, src: &Path, dst: &Path, src_contents_only: bool) -> Result<()> {
    let meta = fs::symlink_metadata(src)
        .map_err(|e| GraphRunError::msg(format!("stat {}: {e}", src.display())))?;
    if meta.is_symlink() {
        let target = fs::read_link(src).map_err(|e| GraphRunError::msg(format!("readlink: {e}")))?;
        if let Some(parent) = dst.parent() {
            mkdir_p_remote(sftp, parent, 0o755)?;
        }
        return sftp.symlink(&target, dst).map_err(ssh_err);
    }
    if meta.is_file() {
        return copy_local_file_to_remote(sftp, src, dst, &meta);
    }
    if !meta.is_dir() {
        return Err(GraphRunError::msg(format!("unsupported local type: {}", src.display())));
    }

    if src_contents_only {
        mkdir_p_remote(sftp, dst, 0o755)?;
        for ent in fs::read_dir(src).map_err(|e| GraphRunError::msg(format!("readdir: {e}")))? {
            let ent = ent.map_err(|e| GraphRunError::msg(format!("readdir: {e}")))?;
            copy_local_tree_to_remote(sftp, &ent.path(), &dst.join(ent.file_name()), false)?;
        }
        return Ok(());
    }

    mkdir_p_remote(sftp, dst, 0o755)?;
    for ent in fs::read_dir(src).map_err(|e| GraphRunError::msg(format!("readdir: {e}")))? {
        let ent = ent.map_err(|e| GraphRunError::msg(format!("readdir: {e}")))?;
        copy_local_tree_to_remote(sftp, &ent.path(), &dst.join(ent.file_name()), false)?;
    }
    Ok(())
}

fn copy_local_to_remote(src_srv: &Server, dst_srv: &Server, spec: &TransferSpec, timeout_ms: u32) -> Result<()> {
    let _ = src_srv;
    let sess = connect_session(dst_srv, timeout_ms)?;
    let sftp = sess.sftp().map_err(ssh_err)?;
    let src = Path::new(&spec.source_path);
    let dst = Path::new(&spec.dest_path);
    copy_local_tree_to_remote(&sftp, src, dst, spec.source_path.ends_with('/'))
}

fn copy_remote_file_to_local(sftp: &Sftp, from: &Path, to: &Path, st: &FileStat) -> Result<()> {
    if let Some(parent) = to.parent() {
        mkdir_p_local(parent, 0o755)?;
    }
    let mut rf = sftp
        .open_mode(from, OpenFlags::READ, 0, OpenType::File)
        .map_err(ssh_err)?;
    let mut wf = fs::File::create(to).map_err(|e| GraphRunError::msg(format!("create {}: {e}", to.display())))?;
    io::copy(&mut rf, &mut wf).map_err(|e| GraphRunError::msg(format!("copy: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(p) = st.perm {
            let _ = fs::set_permissions(to, fs::Permissions::from_mode(p & 0o7777));
        }
    }
    if let Some(m) = st.mtime {
        let ft = filetime::FileTime::from_unix_time(m as i64, 0);
        filetime::set_file_mtime(to, ft).map_err(|e| GraphRunError::msg(format!("set mtime: {e}")))?;
    }
    Ok(())
}

fn copy_remote_tree_to_local(sftp: &Sftp, src: &Path, dst: &Path, src_contents_only: bool) -> Result<()> {
    let st = sftp.lstat(src).map_err(ssh_err)?;
    if is_lnk(&st) {
        let target = sftp.readlink(src).map_err(ssh_err)?;
        if let Some(parent) = dst.parent() {
            mkdir_p_local(parent, 0o755)?;
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, dst).map_err(|e| GraphRunError::msg(format!("symlink: {e}")))?;
            return Ok(());
        }
        #[cfg(not(unix))]
        return Err(GraphRunError::msg("symlink copy to local is only supported on Unix"));
    }
    if is_reg(&st) {
        return copy_remote_file_to_local(sftp, src, dst, &st);
    }
    if !is_dir(&st) {
        return Err(GraphRunError::msg("unsupported remote file type"));
    }

    if src_contents_only {
        mkdir_p_local(dst, 0o755)?;
        for (name, _) in sftp.readdir(src).map_err(ssh_err)? {
            if name == Path::new(".") || name == Path::new("..") {
                continue;
            }
            copy_remote_tree_to_local(sftp, &src.join(&name), &dst.join(&name), false)?;
        }
        return Ok(());
    }

    mkdir_p_local(dst, mode_for_mkdir(&st) as u32)?;
    for (name, _) in sftp.readdir(src).map_err(ssh_err)? {
        if name == Path::new(".") || name == Path::new("..") {
            continue;
        }
        copy_remote_tree_to_local(sftp, &src.join(&name), &dst.join(&name), false)?;
    }
    Ok(())
}

fn copy_remote_to_local(src_srv: &Server, dst_srv: &Server, spec: &TransferSpec, timeout_ms: u32) -> Result<()> {
    let _ = dst_srv;
    let sess = connect_session(src_srv, timeout_ms)?;
    let sftp = sess.sftp().map_err(ssh_err)?;
    let src = Path::new(&spec.source_path);
    let dst = Path::new(&spec.dest_path);
    copy_remote_tree_to_local(&sftp, src, dst, spec.source_path.ends_with('/'))
}

fn copy_local_symlink(from: &Path, to: &Path) -> Result<()> {
    if let Some(p) = to.parent() {
        mkdir_p_local(p, 0o755)?;
    }
    let target = fs::read_link(from).map_err(|e| GraphRunError::msg(format!("readlink: {e}")))?;
    let _ = fs::remove_file(to);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&target, to).map_err(|e| GraphRunError::msg(format!("symlink: {e}")))?;
        Ok(())
    }
    #[cfg(not(unix))]
    Err(GraphRunError::msg("local symlink copy only on Unix"))
}

fn copy_local_tree_to_local(src: &Path, dst: &Path, src_contents_only: bool) -> Result<()> {
    let meta = fs::symlink_metadata(src)
        .map_err(|e| GraphRunError::msg(format!("stat {}: {e}", src.display())))?;
    if meta.is_symlink() {
        return copy_local_symlink(src, dst);
    }
    if meta.is_file() {
        if let Some(p) = dst.parent() {
            mkdir_p_local(p, 0o755)?;
        }
        fs::copy(src, dst).map_err(|e| GraphRunError::msg(format!("copy: {e}")))?;
        #[cfg(unix)]
        {
            let perm = meta.permissions();
            let _ = fs::set_permissions(dst, perm);
        }
        if let Ok(m) = meta.modified() {
            if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
                let ft = filetime::FileTime::from_unix_time(d.as_secs() as i64, 0);
                let _ = filetime::set_file_mtime(dst, ft);
            }
        }
        return Ok(());
    }
    if !meta.is_dir() {
        return Err(GraphRunError::msg("unsupported local file type"));
    }

    if src_contents_only {
        mkdir_p_local(dst, 0o755)?;
        for ent in fs::read_dir(src).map_err(|e| GraphRunError::msg(format!("readdir: {e}")))? {
            let ent = ent.map_err(|e| GraphRunError::msg(format!("readdir: {e}")))?;
            copy_local_tree_to_local(&ent.path(), &dst.join(ent.file_name()), false)?;
        }
        return Ok(());
    }

    mkdir_p_local(dst, 0o755)?;
    for ent in fs::read_dir(src).map_err(|e| GraphRunError::msg(format!("readdir: {e}")))? {
        let ent = ent.map_err(|e| GraphRunError::msg(format!("readdir: {e}")))?;
        copy_local_tree_to_local(&ent.path(), &dst.join(ent.file_name()), false)?;
    }
    Ok(())
}

fn copy_local_to_local(_src_srv: &Server, _dst_srv: &Server, spec: &TransferSpec) -> Result<()> {
    let src = Path::new(&spec.source_path);
    let dst = Path::new(&spec.dest_path);
    copy_local_tree_to_local(src, dst, spec.source_path.ends_with('/'))
}

/// Copy files/directories between two servers (each `kind` may be `local` or `remote`).
/// Trailing slash on `source_path` means “copy directory contents into dest” (like `rsync` source slash).
pub fn run_transfer(bundle: &ConfigBundle, spec: &TransferSpec, timeout_secs: Option<u64>) -> Result<()> {
    let src_srv = bundle.servers.get(&spec.source_server_id).ok_or_else(|| {
        GraphRunError::msg(format!("transfer: unknown source_server_id {:?}", spec.source_server_id))
    })?;
    let dst_srv = bundle.servers.get(&spec.dest_server_id).ok_or_else(|| {
        GraphRunError::msg(format!("transfer: unknown dest_server_id {:?}", spec.dest_server_id))
    })?;

    let timeout_ms: u32 = timeout_secs
        .unwrap_or(300)
        .saturating_mul(1000)
        .min(u64::from(u32::MAX)) as u32;

    match (src_srv.kind.as_str(), dst_srv.kind.as_str()) {
        ("local", "local") => copy_local_to_local(src_srv, dst_srv, spec),
        ("local", "remote") => copy_local_to_remote(src_srv, dst_srv, spec, timeout_ms),
        ("remote", "local") => copy_remote_to_local(src_srv, dst_srv, spec, timeout_ms),
        ("remote", "remote") => copy_remote_to_remote(src_srv, dst_srv, spec, timeout_ms),
        (a, b) => Err(GraphRunError::msg(format!(
            "transfer: unsupported server kind pair ({a}, {b}) (use local or remote)"
        ))),
    }
}
