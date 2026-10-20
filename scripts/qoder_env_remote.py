"""Configure Qoder token counters for remote shell and desktop launches."""
import json
import os
import plistlib
import subprocess
import sys
import tempfile

ENV_NAME = "QODER_EXPOSE_TOKEN_USAGE"
ENV_VALUE = "true"
START = "# >>> Aieyes Qoder token usage >>>"
END = "# <<< Aieyes Qoder token usage <<<"
LABEL = "com.aieyes.qoder-token-usage"


def _run(*args):
    try:
        return subprocess.run(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False).returncode == 0
    except OSError:
        return False


def _atomic(path, data, mode=None):
    directory = os.path.dirname(path) or "."
    os.makedirs(directory, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".aieyes-qoder-", dir=directory)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        if mode is None:
            try:
                mode = os.stat(path).st_mode & 0o777
            except OSError:
                mode = 0o644
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


def _block():
    return (f"{START}\nexport {ENV_NAME}={ENV_VALUE}\n{END}\n").encode("utf-8")


def _merge(path):
    # Keep dotfile symlinks intact; write the real file instead of replacing
    # the link with a regular file.
    if os.path.islink(path):
        path = os.path.realpath(path)
    try:
        with open(path, "rb") as stream:
            old = stream.read()
    except OSError:
        old = b""
    block = _block()
    start, end = START.encode(), END.encode()
    if start in old:
        at = old.index(start)
        tail = old.find(end, at + len(start))
        if tail >= 0:
            tail += len(end)
            if old[tail:tail+2] == b"\r\n":
                tail += 2
            elif old[tail:tail+1] == b"\n":
                tail += 1
            new = old[:at] + block + old[tail:]
        else:
            new = old + (b"" if old.endswith(b"\n") or not old else b"\n") + (b"\n" if old else b"") + block
    else:
        new = old + (b"" if old.endswith(b"\n") or not old else b"\n") + (b"\n" if old else b"") + block
    if new == old:
        return False
    _atomic(path, new)
    return True


def _shell_files(home):
    names = [".profile", ".bashrc", ".zshrc"]
    names.extend(name for name in (".bash_profile", ".bash_login", ".zprofile", ".zshenv")
                 if os.path.isfile(os.path.join(home, name)))
    return [os.path.join(home, name) for name in dict.fromkeys(names)]


def _macos(home):
    directory = os.path.join(home, "Library", "LaunchAgents")
    path = os.path.join(directory, LABEL + ".plist")
    payload = plistlib.dumps({
        "Label": LABEL,
        "ProgramArguments": ["/bin/launchctl", "setenv", ENV_NAME, ENV_VALUE],
        "RunAtLoad": True,
        "StandardOutPath": "/dev/null",
        "StandardErrorPath": "/dev/null",
    }, fmt=plistlib.FMT_XML, sort_keys=False)
    try:
        with open(path, "rb") as stream:
            same = stream.read() == payload
    except OSError:
        same = False
    if not same:
        _atomic(path, payload, 0o644)
    if not _run("/bin/launchctl", "setenv", ENV_NAME, ENV_VALUE):
        return ["launchctl setenv 未能更新当前图形会话"]
    uid = os.getuid()
    domain = "gui/%d" % uid
    _run("/bin/launchctl", "bootout", domain, LABEL)
    if not _run("/bin/launchctl", "bootstrap", domain, path):
        if not _run("/bin/launchctl", "load", "-w", path):
            return ["launchctl 无法加载 Qoder 环境项"]
    return []


def _linux(home):
    directory = os.path.join(home, ".config", "environment.d")
    path = os.path.join(directory, "50-aieyes-qoder.conf")
    content = f"{ENV_NAME}={ENV_VALUE}\n".encode("utf-8")
    try:
        with open(path, "rb") as stream:
            same = stream.read() == content
    except OSError:
        same = False
    if not same:
        _atomic(path, content, 0o644)
    # Best effort for the current user manager; environment.d covers next login.
    _run("systemctl", "--user", "set-environment", f"{ENV_NAME}={ENV_VALUE}")
    return []


def main():
    home = os.path.expanduser("~")
    issues = []
    changed = []
    for path in _shell_files(home):
        try:
            if _merge(path):
                changed.append(path)
        except OSError as error:
            issues.append(f"{path}: {error}")
    if sys.platform == "darwin":
        issues.extend(_macos(home))
    elif sys.platform.startswith("linux"):
        issues.extend(_linux(home))
    else:
        issues.append("当前系统未自动配置图形界面环境；请手动设置 QODER_EXPOSE_TOKEN_USAGE=true。")
    print(json.dumps({"changed": changed, "issues": issues}, ensure_ascii=False))


if __name__ == "__main__":
    main()
