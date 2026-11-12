"""Read her journal back as a story: what she heard, what she did, what she said, and where it went wrong.

    python tools/journal.py                  today's journal
    python tools/journal.py --last 30        only the last 30 minutes
    python tools/journal.py --llm            with every model call under its request (prompt, reply, time)
    python tools/journal.py --request 24     one request, with its model calls in full
    python tools/journal.py <file.jsonl>     another day

    python tools/journal.py --session        the last time she was started, to now: what you pressed and changed in her
                                             windows, what she heard, did and said, in one timeline (--session 2: the
                                             time before)
    python tools/journal.py --all            also every call between her pages and her core, every event, what she
                                             acted out, everything else in the file
    python tools/journal.py --errors         only what went wrong

The options go together: `--session --all --llm` is everything from her last run. Clicks and such are there when she
was started with CAPRI_LOG_EVERYTHING=1. Next to the journal in her logs folder: `llm-*` (every model call in full),
`capricorn-*` (her own log lines), `audio/` (what she heard), `shots/` (pictures models were shown).

Standard library only. Marks: ✗ a failed step, ? an unsure one, ! an error, · a decision she made.
"""

import json
import os
import sys
from datetime import date, datetime, timedelta


def journal_dir():
    return os.path.join(os.environ.get("APPDATA", os.path.expanduser("~/.config")), "capricorn-engram", "logs")


def short(v, n=140):
    s = v if isinstance(v, str) else json.dumps(v, ensure_ascii=False)
    s = s.replace("\n", " ⏎ ")
    return s if len(s) <= n else s[: n - 1] + "…"


def read(path):
    out = []
    if os.path.exists(path):
        for raw in open(path, encoding="utf-8"):
            try:
                out.append(json.loads(raw))
            except json.JSONDecodeError:
                pass
    return out


def model_calls(llm, request, full):
    """Her model calls for one request: who asked, how long, what came back (and with `full`, what was sent)."""
    for c in llm:
        if c.get("kind") != "call" or c.get("request") != request:
            continue
        reply = c.get("reply") or {}
        calls = ", ".join(f"{x['name']} {short(x.get('args'), 80)}" for x in reply.get("calls", []))
        usage = reply.get("usage") or {}
        print(f"           ◇ {c.get('role')} · {c.get('model')} · {c.get('ms')} ms · {usage.get('input', 0)}→{usage.get('output', 0)} tokens")
        if full:
            for m in c.get("msgs", []):
                print(f"               {m.get('role')}{' (' + m['tool'] + ')' if m.get('tool') else ''}: {short(m.get('text', ''), 400)}")
                for x in m.get("calls", []):
                    print(f"                 → {x['name']} {short(x.get('args'), 200)}")
        if c.get("error"):
            print(f"             ! {short(c['error'], 300)}")
        elif calls:
            print(f"             → {calls}")
        elif reply.get("text"):
            print(f"             “{short(reply['text'], 300 if full else 160)}”")


def ui_line(e, everything):
    """One thing that happened in one of her windows, or None when it's only for --all."""
    ev, page = e.get("ev"), e.get("page", "")
    where = e.get("text") or e.get("title") or e.get("card") or e.get("on", "")
    if ev in ("click", "dblclick", "contextmenu"):
        return f"🖱 {ev if ev != 'click' else 'pressed'} “{short(where, 70)}”  [{page} {e.get('on', '')}]"
    if ev == "change":
        return f"✎ {short(e.get('field') or where, 60)} → {short(e.get('value'), 90)}  [{page} {e.get('on', '')}]"
    if ev == "keydown":
        return f"⌨ {e.get('key')}  [{page}]"
    if ev == "panel":
        rest = {k: v for k, v in e.items() if k not in ("t", "kind", "page", "ev", "op")}
        return f"▣ panel {e.get('op')}{': ' + short(rest, 200) if rest else ''}  [{page}]"
    if ev == "thrown":
        return f"🤾 thrown at {e.get('speed')} px/s{' (by you)' if e.get('byYou') else ''}  [{page}]"
    if ev == "error":
        return f"! page error ({e.get('source')}): {short(e.get('message') or e.get('what'), 300)}  {e.get('where', '')}  [{page}]"
    if ev == "console" and e.get("level") in ("error", "warn"):
        return f"! console {e.get('level')}: {short(e.get('text'), 300)}  [{page}]"
    if ev == "ipc" and e.get("failed"):
        return f"! {e.get('cmd')} failed: {short(e.get('failed'), 300)}  args {short(e.get('args'), 120)}  [{page}]"
    if not everything:
        return None
    if ev == "ipc":
        return f"↔ {e.get('cmd')} {short(e.get('args', ''), 120)} → {short(e.get('result', ''), 120)} ({e.get('ms')} ms)  [{page}]"
    if ev == "event":
        return f"⇠ {e.get('event')} {short(e.get('payload', ''), 150)}  [{page}]"
    if ev in ("act", "state"):
        return f"🎭 {ev} {e.get('name')}  [{page}]"
    if ev == "page":
        return f"🗔 {e.get('state')} {e.get('url', '')}  [{page}]"
    if ev == "console":
        return f"  console {e.get('level')}: {short(e.get('text'), 200)}  [{page}]"
    return f"  {ev} {short({k: v for k, v in e.items() if k not in ('t', 'kind', 'page', 'ev')}, 200)}  [{page}]"


def went_wrong(e):
    k = e.get("kind")
    if k in ("error", "panic", "native crash", "cancelled", "brain waiting"):
        return True
    if k == "step":
        return e.get("step") == "finished" and e.get("status") in ("failed", "unsure")
    if k == "ui":
        return e.get("ev") == "error" or (e.get("ev") == "console" and e.get("level") in ("error", "warn")) or (e.get("ev") == "ipc" and bool(e.get("failed")))
    return False


def main():
    args = sys.argv[1:]
    minutes = only = None
    show_llm = "--llm" in args
    if show_llm:
        args.remove("--llm")
    everything = "--all" in args
    if everything:
        args.remove("--all")
    errors = "--errors" in args
    if errors:
        args.remove("--errors")
    session = 0
    if "--session" in args:
        i = args.index("--session")
        session = 1
        del args[i]
        if i < len(args) and args[i].isdigit():
            session = int(args[i])
            del args[i]
    if "--last" in args:
        i = args.index("--last")
        minutes = int(args[i + 1])
        del args[i : i + 2]
    if "--request" in args:
        i = args.index("--request")
        only = int(args[i + 1])
        del args[i : i + 2]
        show_llm = True
    path = args[0] if args else os.path.join(journal_dir(), f"journal-{date.today():%Y-%m-%d}.jsonl")
    if not os.path.exists(path):
        sys.exit(f"no journal at {path}")
    llm = read(os.path.join(os.path.dirname(path), os.path.basename(path).replace("journal-", "llm-"))) if show_llm else []
    since = (datetime.now() - timedelta(minutes=minutes)).strftime("%H:%M:%S") if minutes else None
    events = read(path)
    if session:
        starts = [i for i, e in enumerate(events) if e.get("kind") == "start"]
        if len(starts) < session:
            sys.exit(f"only {len(starts)} starts in {path}")
        begin = starts[-session]
        end = starts[-session + 1] if session > 1 else len(events)
        events = events[begin:end]
    known = {"start", "heard", "wake", "barge_in", "listening", "timeout", "heard_request", "hushed", "cancelled", "request", "step", "note", "answer", "voice", "hush", "confirm", "game", "game_mode", "screen", "wake_word", "error", "mic", "ui", "session", "settings", "tray", "hotkey", "panic", "native crash", "online brain", "restart", "quit"}
    for e in events:
        t, k = e.get("t", "")[:8], e.get("kind")
        if errors and not went_wrong(e):
            continue
        if since and t < since:
            continue
        if only is not None and e.get("id") != only:
            continue
        if k == "start":
            print(f"\n=== {t} she started ({'dev' if e.get('dev') else 'release'} {e.get('version')})")
        elif k == "heard":
            verdict = f"  → {e['verdict']}" if e.get("verdict") else ""
            print(f"{t}   heard: “{e.get('text')}”  ({e.get('ms')} ms){verdict}{'  🎧 ' + e['audio'] if e.get('audio') else ''}")
        elif k == "wake":
            print(f"{t}   → her name: listening")
        elif k == "barge_in":
            print(f"{t}   → her name over her voice (“{short(e.get('heard', ''), 60)}”, read in {e.get('ms')} ms): she stops, listening")
        elif k == "listening":
            print(f"{t}   → listening: {e.get('why')}")
        elif k == "timeout":
            print(f"{t}   → stopped listening (nothing came)")
        elif k == "heard_request":
            print(f"{t}   → request{' (with her name)' if e.get('after_name') else ''}: “{e.get('text')}”")
        elif k == "hushed":
            print(f"{t}   → “{e.get('text')}” right after cutting her off: quiet, nothing else")
        elif k == "cancelled":
            print(f"{t}   ✗ dropped “{short(e.get('text', ''), 60)}”: {e.get('why')}")
        elif k == "request":
            print(f"{t} #{e.get('id')} {'🎙' if e.get('spoken') else '⌨'}  {e.get('text')}")
        elif k == "step":
            step = e.get("step")
            if step == "finished":
                mark = {"ok": "✓", "failed": "✗", "unsure": "?"}.get(e.get("status"), " ")
                print(f"{t}      {mark} {e.get('tool')}: {short(e.get('summary', ''))}")
            elif step == "started":
                print(f"{t}      ▸ {e.get('tool')} {short(e.get('args'), 120)}")
        elif k == "note":
            data = e.get("data")
            print(f"{t}      · {e.get('what')}{'  ' + short(data, 160) if data else ''}")
        elif k == "answer":
            print(f"{t} #{e.get('id')} ⇒ [{e.get('via')} {e.get('model') or ''} {e.get('ms')} ms] “{e.get('line')}”")
            for s in e.get("steps", []):
                print(f"           {s.get('tool')} {short(s.get('args'), 120)} → {s.get('status')}: {short(s.get('summary', ''), 120)}")
                if s.get("data"):
                    print(f"             saw: {short(s['data'], 300)}")
            if show_llm:
                model_calls(llm, e.get("id"), only is not None)
        elif k == "voice":
            if e.get("text"):
                print(f"{t}   🔊 says: “{short(e.get('text'))}”")
            elif e.get("started") and e.get("read_as"):
                print(f"{t}   🔊 read as: “{short(e.get('read_as'))}”")
            elif e.get("finished") and e.get("cut"):
                print(f"{t}   🔊 cut off")
        elif k == "hush":
            print(f"{t}   🔇 stopped talking: {e.get('why')}")
        elif k == "confirm":
            print(f"{t}   asked to confirm: {short(e.get('what'))} → {'yes' if e.get('yes') else 'no'}")
        elif k == "game":
            print(f"{t}   🎮 game {'started' if e.get('on') else 'closed'}: {e.get('exe')} ({e.get('title')}) mode {e.get('mode')}")
        elif k == "game_mode":
            print(f"{t}   🎮 {e.get('exe')}: {e.get('mode')}")
        elif k == "screen":
            print(f"{t}   moved {e.get('from')} → {e.get('to')}")
        elif k == "wake_word":
            print(f"{t}   listening for her name: {'on' if e.get('on') else 'off'}")
        elif k == "error":
            print(f"{t}   ! {e.get('where')}: {e.get('message')}")
        elif k == "mic":
            print(f"{t}   mic {'open: ' + e.get('name', '') if e.get('open') else 'closed'}")
        elif k == "ui":
            line = ui_line(e, everything)
            if line:
                print(f"{t}   {line}")
        elif k == "session":
            brain = (e.get("settings") or {}).get("brain") or {}
            print(f"{t}   running {e.get('commit')} ({e.get('uncommitted_files')} files uncommitted)")
            print(f"{t}   brains: quick {brain.get('fast')}, hard jobs {brain.get('smart')}, eyes {brain.get('eyes')}, online {'allowed' if brain.get('cloud') else 'off'}")
        elif k == "settings":
            for name, (old, new) in (e.get("changed") or {}).items():
                print(f"{t}   ⚙ setting {name}: {short(old, 60)} → {short(new, 60)}")
            if e.get("restart"):
                print(f"{t}   ⚙ (needs a restart to apply)")
        elif k == "online brain":
            print(f"{t}   ⚙ online brain checked and picked: {e.get('provider')} → {e.get('fast')} (eyes {e.get('eyes')})")
        elif k == "tray":
            print(f"{t}   🖱 tray menu: {e.get('item')}")
        elif k == "hotkey":
            print(f"{t}   ⌨ hotkey Ctrl+Alt+{str(e.get('key', '')).replace('Key', '')}")
        elif k == "panic":
            print(f"{t}   ‼ CRASH in {e.get('thread')}: {short(e.get('message'), 600)}")
            print(short(e.get("backtrace", ""), 1800).replace(" ⏎ ", "\n"))
        elif k == "native crash":
            print(f"{t}   ‼ NATIVE CRASH {e.get('code')} at {e.get('address')} in {e.get('thread')}")
        elif k in ("restart", "quit"):
            print(f"{t}   ⏻ {k} ({e.get('from')})")
        elif k not in known and (everything or session):
            print(f"{t}   {k}: {short({x: y for x, y in e.items() if x not in ('t', 'kind')}, 220)}")


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    main()
