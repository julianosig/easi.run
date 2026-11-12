#!/usr/bin/env python3
"""The hero's film, narrated, in every language the site speaks.

script/<locale>.json holds a language's narration (`parts`, one sentence or question each, in the order of stage.html's
stations), the words each part highlights (`marks`) and, except English, its on-screen labels (`labels`, English to
the language). stage.html is the film: one 1920x1080 page whose render(t) draws any moment. audio/ holds its music, a
riser and the logo's impact (made with ElevenLabs).

From the repository's root, each step for every language, or those --locale= names:

  site/film/make.py translate     script/<locale>.json from English (OPENAI_API_KEY), after the English changed
  site/film/make.py voice <out>   the narration and when each word is said (ELEVENLABS_API_KEY, OPENAI_API_KEY)
  site/film/make.py film <out>    the frames, the mix and the files the landing plays (Docker)

then site/deploy/film.sh <out> publishes <out>/<locale>/<locale>.{webm,mp4,jpg,avif}.
"""
import concurrent.futures
import difflib
import html
import json
import os
import re
import shutil
import subprocess
import sys
import unicodedata
import urllib.error
import urllib.request
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
LOCALES = re.findall(r"^  '?([a-zA-Z-]+)'?: '", (ROOT / 'i18n/locales.ts').read_text(), re.M)
ISO = {locale: locale.split('-')[0] for locale in LOCALES}
NAMES = {'pt-BR': 'Brazilian Portuguese', 'es': 'Spanish (neutral Latin American)', 'zh-CN': 'Simplified Chinese',
         'hi': 'Hindi', 'fr': 'French', 'de': 'German', 'ja': 'Japanese', 'ru': 'Russian', 'id': 'Indonesian',
         'ko': 'Korean'}
# Languages written without spaces: their parts carry spaces only where a line may break, and none when spoken.
UNSPACED = {'zh-CN', 'ja'}
# The scripts Instrument Sans lacks, each language's font (fetched once into <out>/fonts/).
NOTO = 'https://github.com/notofonts'
SCRIPT_FONT = {
    'zh-CN': ('noto-sc.otf', f'{NOTO}/noto-cjk/raw/main/Sans/Variable/OTF/NotoSansCJKsc-VF.otf'),
    'ja': ('noto-jp.otf', f'{NOTO}/noto-cjk/raw/main/Sans/Variable/OTF/NotoSansCJKjp-VF.otf'),
    'ko': ('noto-kr.otf', f'{NOTO}/noto-cjk/raw/main/Sans/Variable/OTF/NotoSansCJKkr-VF.otf'),
    'hi': ('noto-devanagari.woff2', 'https://cdn.jsdelivr.net/npm/@fontsource-variable/noto-sans-devanagari@5.2.8/files/noto-sans-devanagari-devanagari-wght-normal.woff2'),
    'ru': ('noto-cyrillic.woff2', 'https://cdn.jsdelivr.net/npm/@fontsource-variable/noto-sans@5.2.10/files/noto-sans-cyrillic-wght-normal.woff2'),
}
VOICE = 'TX3LPaxmHKxFdv7VOQHJ'  # Liam, ElevenLabs
VOICE_MODEL = 'eleven_v4_turbo'
FFMPEG = 'linuxserver/ffmpeg@sha256:822d04f0b2d0c4847430f5cc90f20ea48be866bb502ece63fdeb729400bf5361'
PLAYWRIGHT = 'mcr.microsoft.com/playwright:v1.65.0-noble@sha256:9beb546099e11f52d62e505537914870e01a67e0e6b00ae41da4775324e73dbe'
FPS = 60  # captured; the files play at 30, two frames blended into each


def script(locale):
    return json.loads((HERE / 'script' / f'{locale}.json').read_text())


def post(url, body, headers, raw=False):
    request = urllib.request.Request(url, data=body if raw else json.dumps(body).encode(), headers=headers)
    with urllib.request.urlopen(request, timeout=900) as response:
        return response.read()


def ffmpeg(out, command):
    """Runs a shell command in the ffmpeg image, <out> mounted at /o and this directory at /film."""
    return subprocess.run(['docker', 'run', '--rm', '--user', f'{os.getuid()}:{os.getgid()}', '-v', f'{out}:/o',
                           '-v', f'{HERE}:/film:ro', '--entrypoint', 'sh', FFMPEG, '-c', f'set -e; {command}'],
                          check=True, capture_output=True, text=True).stdout.strip()


def openai(prompt, effort):
    body = {'model': 'gpt-6.1-sol', 'input': prompt, 'reasoning': {'effort': effort},
            'text': {'format': {'type': 'json_object'}}}
    answer = json.loads(post('https://api.openai.com/v1/responses', body,
                             {'Authorization': f'Bearer {os.environ["OPENAI_API_KEY"]}',
                              'Content-Type': 'application/json'}))
    return json.loads(''.join(c.get('text', '') for o in answer['output'] if o['type'] == 'message'
                              for c in o['content']))


def translate(locale):
    english = script('en')
    space = ('For display, put a single ASCII space between natural phrase chunks of 2 to 6 characters in each '
             'narration line (the spaces only mark where the on-screen text may break and are removed before '
             'speech). Labels get no such spaces.\n' if locale in UNSPACED else '')
    prompt = f"""Translate a product video's narration and on-screen labels from English into {NAMES[locale]}.

The narration is read aloud by a voice actor over a fast motion-design film about easi.run, a self-hosted Chrome that agents drive. Make each line sound natural, punchy and conversational in {NAMES[locale]}, as a native copywriter would say it, not a literal translation. Keep every fact exactly: add no claim, drop no item, keep each list complete (for example every pain in a question, every feature named). The first five lines are questions to the viewer and must stay questions. Keep each line about as long as the English when spoken.

Keep these exactly as written, untranslated: easi.run, Chrome, Claude Code, Codex, MCP, CDP, Playwright, Puppeteer, Selenium, Slack, Discord, 2FA, CAPTCHA, AGPL-3.0, Linux, file names, bank.example. Say "agents" with the plain word for agent, never "AI agents".
{space}Labels are short UI texts in the film (buttons, chips, window titles, list rows); keep them short, and use the admin's own words where it has them (client/messages/{locale}/).

Return only JSON: {{"parts": [{len(english['parts'])} strings, in order], "labels": {{"<English label>": "<translation>", ... every label}}}}.

Narration lines:
{json.dumps(english['parts'], ensure_ascii=False)}

Labels:
{json.dumps(english['labels'], ensure_ascii=False)}"""
    done = openai(prompt, 'medium')
    assert len(done['parts']) == len(english['parts']) and set(done['labels']) >= set(english['labels']), locale
    tokens = [[w for w in p.split(' ') if w] for p in done['parts']]
    marks = openai('For each subtitle line of a product film, choose the tokens to highlight. The English version '
                   'highlights these words (pairs of English line and its highlighted words):\n'
                   + json.dumps(list(zip(english['parts'], english['marks'])), ensure_ascii=False)
                   + '\n\nThe translated lines, split into tokens:\n' + json.dumps(tokens, ensure_ascii=False)
                   + '\n\nFor each translated line, return the tokens (copied exactly from its token list) that carry '
                   'the meaning of the English highlights, 2 to 8 per line. Return only JSON: {"marks": [[...], ...]}.',
                   'low')['marks']
    done['marks'] = [[w for w in m if w in tokens[i]] for i, m in enumerate(marks)]
    (HERE / 'script' / f'{locale}.json').write_text(
        json.dumps({'parts': done['parts'], 'marks': done['marks'], 'labels': done['labels']}, ensure_ascii=False,
                   indent=1) + '\n')
    return locale


def letters(text):
    return [c for c in unicodedata.normalize('NFKC', text).lower() if c.isalnum()]


def voice(out, locale):
    """<out>/<locale>/voice.wav, and timing.json: when each shown word is said, aligned letter by letter."""
    d = out / locale
    d.mkdir(parents=True, exist_ok=True)
    parts = script(locale)['parts']
    said = [p.replace(' ', '') if locale in UNSPACED else p for p in parts]
    text = ('' if locale in UNSPACED else ' ').join(said).replace('easi.run', 'easy dot run')
    headers = {'xi-api-key': os.environ['ELEVENLABS_API_KEY'], 'Content-Type': 'application/json'}
    body = {'text': text, 'model_id': VOICE_MODEL, 'language_code': ISO[locale],
            'voice_settings': {'stability': 0.45, 'similarity_boost': 0.8, 'style': 0.35, 'use_speaker_boost': True,
                               'speed': 1.1}}
    (d / 'raw.mp3').write_bytes(post(f'https://api.elevenlabs.io/v1/text-to-speech/{VOICE}?output_format=mp3_44100_192',
                                     body, headers))
    o = f'/o/{locale}'
    raw = float(ffmpeg(out, f'ffmpeg -v error -y -i {o}/raw.mp3 {o}/raw.wav; '
                            f'ffprobe -v error -show_entries format=duration -of csv=p=0 {o}/raw.wav'))
    tempo = max(1.0, min(1.15, raw / 90))
    total = float(ffmpeg(out, f'ffmpeg -v error -y -i {o}/raw.wav -af "silenceremove=start_periods=1:start_threshold=-45dB:'
                              f'start_silence=0.03,atempo={tempo:.4f},adelay=350|350,aresample=48000,loudnorm=I=-16:'
                              f'TP=-1.5:LRA=7" -ar 48000 {o}/voice.wav; '
                              f'ffprobe -v error -show_entries format=duration -of csv=p=0 {o}/voice.wav'))
    boundary = uuid.uuid4().hex
    fields = [('model', 'whisper-1'), ('response_format', 'verbose_json'), ('timestamp_granularities[]', 'word'),
              ('language', ISO[locale])]
    form = b''.join(f'--{boundary}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode() for k, v in fields)
    form += (f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="voice.wav"\r\n'
             'Content-Type: audio/wav\r\n\r\n').encode() + (d / 'voice.wav').read_bytes() + f'\r\n--{boundary}--\r\n'.encode()
    heard = json.loads(post('https://api.openai.com/v1/audio/transcriptions', form,
                            {'Authorization': f'Bearer {os.environ["OPENAI_API_KEY"]}',
                             'Content-Type': f'multipart/form-data; boundary={boundary}'}, raw=True))['words']
    heard_letters, heard_at = [], []
    for w in heard:
        cs = letters(w['word'])
        for i, c in enumerate(cs):
            heard_letters.append(c)
            heard_at.append(w['start'] + (w['end'] - w['start']) * i / len(cs))
    shown = []  # every shown word: [text, part]
    for i, p in enumerate(parts):
        for w in p.split(' '):
            if not w:
                continue
            # Latin words in an unspaced language stay one piece on screen ("Claude Code").
            if locale in UNSPACED and shown and shown[-1][1] == i and re.search(r'[A-Za-z0-9]$', shown[-1][0]) \
                    and re.match(r'[A-Za-z]', w):
                shown[-1][0] += ' ' + w
            else:
                shown.append([w, i])
    shown_letters, owner = [], []
    for n, (w, _) in enumerate(shown):
        for c in letters(w.replace('easi.run', 'easy dot run')):
            shown_letters.append(c)
            owner.append(n)
    at = [None] * len(shown_letters)
    for a, b, n in difflib.SequenceMatcher(None, shown_letters, heard_letters, autojunk=False).get_matching_blocks():
        for k in range(n):
            at[a + k] = heard_at[b + k]
    known = [i for i, x in enumerate(at) if x is not None]
    for i in range(len(at)):
        if at[i] is None:
            before = max((k for k in known if k < i), default=None)
            after = min((k for k in known if k > i), default=None)
            at[i] = (at[after] if before is None else at[before] + 0.08 * (i - before) if after is None
                     else at[before] + (at[after] - at[before]) * (i - before) / (after - before))
    times = []
    for n in range(len(shown)):
        first = owner.index(n) if n in owner else None
        times.append(at[first] if first is not None else (times[-1] + 0.05 if times else 0.0))
        if len(times) > 1:
            times[-1] = max(times[-1], times[-2])
    timing = {'end': round(total + 1.9, 3), 'parts': []}
    for i in range(len(parts)):
        words = [{'w': w, 't': round(times[n], 3)} for n, (w, p) in enumerate(shown) if p == i]
        timing['parts'].append({'t': words[0]['t'], 'end': words[-1]['t'] + 0.35, 'words': words})
    (d / 'timing.json').write_text(json.dumps(timing, ensure_ascii=False))
    return f'{locale}: {total:.1f} s, {len(known)} of {len(shown_letters)} letters heard'


def stage(out, locale):
    """<out>/<locale>/film.html: the stage in that language, its timing, highlights, labels and script font."""
    s = (HERE / 'stage.html').read_text()
    words = script(locale)
    timing = json.loads((out / locale / 'timing.json').read_text())
    s = s.replace('__TIMING__', json.dumps(timing, ensure_ascii=False)).replace('__LANG__', locale)
    s = s.replace('__MARK__', json.dumps(words['marks'], ensure_ascii=False)).replace('url(fonts/', 'url(../fonts/')
    labels = {} if locale == 'en' else words['labels']  # English's are the stage's own
    for english, label in sorted(labels.items(), key=lambda x: -len(x[0])):
        shown = html.escape(label, quote=False).replace("'", '&#39;')
        found = s.count(f"'> {english}'")
        s = s.replace(f"'> {english}'", f"'> {shown}'")
        s, n = re.subn(r'>(\s*)' + re.escape(html.escape(english, quote=False)) + r'(\s*)<',
                       lambda m: f'>{m.group(1)}{shown}{m.group(2)}<', s)
        found += n
        s, n = re.subn(r"(?<=')" + re.escape(english) + r"(?=')", lambda m: shown, s)
        assert found + n, f'{locale}: the label {english!r} is not on the stage'
    if locale in SCRIPT_FONT:
        file, _ = SCRIPT_FONT[locale]
        kind = 'opentype' if file.endswith('.otf') else 'woff2'
        s = s.replace('<style>', f"<style>\n@font-face{{font-family:'SCRIPT';src:url(../fonts/{file}) format('{kind}');"
                                 'font-weight:100 900}', 1)
        s = s.replace("font-family:'IS',sans-serif", "font-family:'IS','SCRIPT',sans-serif")
        s = s.replace("font-family:'GM',monospace", "font-family:'GM','SCRIPT',monospace")
        s = s.replace('font-family:IS;', "font-family:IS,'SCRIPT';")
        sample = json.dumps(''.join(sorted({c for p in timing['parts'] for w in p['words'] for c in w['w']}
                                           | {c for v in labels.values() for c in v})), ensure_ascii=False)
        s = s.replace('window.ready=document.fonts.load("500 66px IS")',
                      f'window.ready=Promise.all([400,500,600,700].map(w=>document.fonts.load(w+" 66px SCRIPT",{sample})))'
                      '.then(()=>document.fonts.load("500 66px IS"))')
    if locale in UNSPACED:
        s = s.replace('.w{display:inline-block;position:relative;margin-right:.26em;',
                      '.w{display:inline-block;position:relative;margin-right:.04em;')
    # A part's size also fits its longest word in the column.
    per = 1.02 if locale in ('zh-CN', 'ja', 'ko') else 0.58
    s = s.replace("d.style.fontSize=size+'px';",
                  f"const fit=Math.floor(740/(Math.max(...p.words.map(x=>x.w.length))*{per}));"
                  "d.style.fontSize=Math.min(size,fit)+'px';")
    (out / locale / 'film.html').write_text(s)


def fonts(out):
    d = out / 'fonts'
    d.mkdir(parents=True, exist_ok=True)
    for name in ('instrument-sans', 'geist-mono'):
        source = ROOT / f'site/node_modules/@fontsource-variable/{name}/files/{name}-latin-wght-normal.woff2'
        shutil.copyfile(source, d / source.name)
    for file, url in SCRIPT_FONT.values():
        if not (d / file).exists():
            (d / file).write_bytes(urllib.request.urlopen(url, timeout=600).read())


def playwright(out, command):
    subprocess.run(['docker', 'run', '--rm', '--user', f'{os.getuid()}:{os.getgid()}', '-e', 'HOME=/tmp',
                    '-v', f'{ROOT}:/w', '-v', f'{out}:{out}', '-w', '/w', PLAYWRIGHT, 'bash', '-c', command], check=True)


def render(out, locale, workers):
    playwright(out, f'node site/film/render.mjs {out} {locale} {workers} {FPS}')


def encode(out, locale):
    """The mix (the voice over the music, which ducks under it, the riser and the logo's impact) and the files."""
    timing = json.loads((out / locale / 'timing.json').read_text())
    last = timing['parts'][-1]['words']
    logo = last[-2]['t'] - 0.25  # stage.html's end card comes in then
    end = timing['end']
    # The music's drop (at 18 s) lands on the answer after the questions, its end just before the logo.
    rate = max(0.8, min(1.2, 76 / ((logo - 0.3) - timing['parts'][5]['t'])))
    start = 18 - timing['parts'][5]['t'] * rate
    o = f'/o/{locale}'
    blend = 'tmix=frames=2,fps=30,format=yuv420p'
    ffmpeg(out, f'''ffmpeg -v error -y -i {o}/voice.wav -i /film/audio/music.mp3 -i /film/audio/riser.mp3 -i /film/audio/impact.mp3 -filter_complex "[1:a]atrim=start={start:.3f},asetpts=PTS-STARTPTS,atempo={rate:.4f},aresample=48000,volume=-9dB,afade=t=in:d=0.3,afade=t=out:st={logo - 2.1:.3f}:d=1.9[m];[0:a]aresample=48000,apad,asplit=2[v][sc];[m][sc]sidechaincompress=threshold=0.025:ratio=6:attack=15:release=400[md];[2:a]aresample=48000,volume=-8dB,adelay={int((logo - 2.5) * 1000)}:all=1[r];[3:a]aresample=48000,volume=-4dB,adelay={int((logo + 0.05) * 1000)}:all=1[i];[v][md][r][i]amix=inputs=4:duration=first:normalize=0,atrim=end={end},afade=t=out:st={end - 0.6:.3f}:d=0.6,loudnorm=I=-15:TP=-1.2:LRA=9[a]" -map "[a]" -ar 48000 {o}/mix.wav
ffmpeg -v error -y -framerate {FPS} -i {o}/frames/%05d.jpg -i {o}/mix.wav -vf "{blend}" -map 0:v -map 1:a -c:v libsvtav1 -preset 6 -crf 36 -g 300 -c:a libopus -b:a 128k -shortest {o}/{locale}.webm
ffmpeg -v error -y -framerate {FPS} -i {o}/frames/%05d.jpg -i {o}/mix.wav -vf "{blend}" -map 0:v -map 1:a -c:v libx265 -preset medium -crf 28 -tag:v hvc1 -x265-params log-level=error -c:a aac -b:a 128k -movflags +faststart -shortest {o}/{locale}.mp4
ffmpeg -v error -y -ss {timing['parts'][0]['t'] + 1.7:.2f} -i {o}/{locale}.mp4 -frames:v 1 -vf scale=1280:-2 -q:v 3 {o}/{locale}.jpg
ffmpeg -v error -y -framerate {FPS} -i {o}/frames/%05d.jpg -vf "fps=15,scale=960:-2:flags=lanczos,format=yuv420p" -c:v libsvtav1 -preset 6 -crf 46 -loop 0 {o}/film.avif
mv {o}/film.avif {o}/{locale}.avif''')
    return locale


def main():
    step = sys.argv[1]
    chosen = next((a.split('=', 1)[1].split(',') for a in sys.argv if a.startswith('--locale=')), LOCALES)
    pool = concurrent.futures.ThreadPoolExecutor(len(chosen))
    if step == 'translate':
        jobs = [pool.submit(translate, locale) for locale in chosen if locale != 'en']
    else:
        out = Path(sys.argv[2]).resolve()
        if step == 'voice':
            jobs = [pool.submit(voice, out, locale) for locale in chosen]
        elif step == 'film':
            fonts(out)
            for locale in chosen:
                stage(out, locale)
            # Playwright once, before the languages render at once from it.
            playwright(out, 'npm ci -s --prefix bench/runner --cache /tmp/npm')
            # Half the processors, shared between the languages rendering at once.
            workers = max(1, (os.cpu_count() or 2) // 2 // len(chosen))
            list(pool.map(lambda locale: render(out, locale, workers), chosen))
            # Three encodes at a time: each holds tens of gigabytes of frames in flight.
            encoders = concurrent.futures.ThreadPoolExecutor(3)
            jobs = [encoders.submit(encode, out, locale) for locale in chosen]
        else:
            sys.exit(__doc__)
    for job in concurrent.futures.as_completed(jobs):
        print(job.result())


if __name__ == '__main__':
    main()
