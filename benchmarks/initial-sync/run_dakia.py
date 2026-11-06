"""Kill a compiled Dakia benchmark at the measured pl_mail time limit."""
import argparse
import json
import os
import signal
import subprocess
import time

p = argparse.ArgumentParser()
p.add_argument('--binary', required=True)
p.add_argument('--pl-result', required=True)
p.add_argument('--output', required=True)
p.add_argument('--cert', required=True)
p.add_argument('--port', default='1993')
p.add_argument('--count', default='50000')
p.add_argument('--engine-only', action='store_true')
p.add_argument('--resources')
p.add_argument('--resource-interval', type=float, default=.25)
a = p.parse_args()
with open(a.pl_result) as f:
    results = [json.loads(line) for line in f if line.startswith('{')]
limit = results[-1]['seconds']
env = os.environ | dict(BENCH_CERT=a.cert, BENCH_PORT=a.port, BENCH_COUNT=a.count,
                       BENCH_MAX_SECONDS=str(limit), BENCH_SCHEDULED='0' if a.engine_only else '1')
with open(a.output, 'w') as output:
    started = time.monotonic()
    process = subprocess.Popen([a.binary, 'initial_mailbox_sync_benchmark', '--ignored', '--nocapture'],
                               env=env, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
    monitor = None
    status = -1
    try:
        if a.resources:
            from resource_usage import NativeMonitor
            monitor = NativeMonitor(process.pid, a.resources, a.resource_interval)
            monitor.start()
        status = process.wait(timeout=max(0, limit - (time.monotonic() - started)))
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        output.write(f'\nSTOPPED: exceeded pl_mail duration of {limit:.6f} seconds\n')
        status = 124
    except BaseException:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        raise
    finally:
        if monitor and hasattr(monitor, 'thread') and monitor.thread.ident is not None:
            monitor.finish(status)
raise SystemExit(status)
