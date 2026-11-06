"""Resource sampling for isolated sync workers; fake IMAP and monitor excluded.

CPU: 100% is one logical core. Docker CPU counters cover app + DB cgroups.
Native CPU/rusage covers the child process and its embedded SQLite workers.
Memory accounting is explicitly labelled because Darwin and Linux differ.
"""
import ctypes
import http.client
import json
import os
from pathlib import Path
import platform
import resource
import socket
import threading
import time


class UnixHTTP(http.client.HTTPConnection):
    def __init__(self, path):
        super().__init__('localhost', timeout=10)
        self.path = path

    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(self.path)


def weighted_memory(samples, field):
    area = duration = 0.0
    for before, after in zip(samples, samples[1:]):
        dt = after['elapsed_seconds'] - before['elapsed_seconds']
        area += (before[field] + after[field]) * .5 * dt
        duration += dt
    return area / duration if duration else samples[0][field]


def summarize(samples, memory_fields, cpu_seconds=None, wall_seconds=None):
    wall = wall_seconds or samples[-1]['elapsed_seconds'] - samples[0]['elapsed_seconds']
    cpu = cpu_seconds if cpu_seconds is not None else (samples[-1]['cpu_total_ns'] - samples[0]['cpu_total_ns']) / 1e9
    peaks = []
    for before, after in zip(samples, samples[1:]):
        dt = after['elapsed_seconds'] - before['elapsed_seconds']
        if dt > 0:
            peaks.append(100 * (after['cpu_total_ns'] - before['cpu_total_ns']) / 1e9 / dt)
    result = dict(wall_seconds=wall, cpu_core_seconds=cpu,
                  average_cpu_percent_one_core=100 * cpu / wall,
                  peak_sample_cpu_percent_one_core=max(peaks, default=0),
                  samples=len(samples))
    for field in memory_fields:
        result['peak_' + field] = max(s[field] for s in samples)
        result['average_' + field] = weighted_memory(samples, field)
    return result


class Monitor:
    def __init__(self, output, interval=.25):
        self.output = Path(output)
        self.interval = interval
        self.samples = []
        self.errors = []
        self.stop_event = threading.Event()

    def start(self):
        self.started = time.monotonic()
        self.take_sample()
        self.thread = threading.Thread(target=self.loop, daemon=True)
        self.thread.start()

    def take_sample(self):
        try:
            sample = self.snapshot()
            if sample is not None:
                sample['elapsed_seconds'] = time.monotonic() - self.started
                self.samples.append(sample)
        except Exception as error:
            self.errors.append(repr(error))

    def loop(self):
        while not self.stop_event.wait(self.interval):
            self.take_sample()

    def finish(self, status):
        self.stop_event.set()
        self.thread.join()
        self.take_sample()
        self.wall = time.monotonic() - self.started
        if not self.samples:
            raise RuntimeError('No resource samples collected: ' + repr(self.errors))
        result = self.result()
        result.update(exit_status=status, sample_interval_seconds=self.interval,
                      sampling_errors=self.errors,
                      scope='sync job lifecycle, including worker startup and validation; excludes fixture and sampler')
        self.output.write_text(json.dumps(result, indent=2) + '\n')
        self.output.with_suffix('.samples.jsonl').write_text(''.join(json.dumps(s) + '\n' for s in self.samples))
        if self.errors:
            raise RuntimeError('Resource sampling failed: ' + repr(self.errors))
        return result


class DockerMonitor(Monitor):
    def __init__(self, containers, output, interval=.25, socket_path=None):
        super().__init__(output, interval)
        self.containers = containers
        path = socket_path or os.environ.get('BENCH_DOCKER_SOCKET')
        if not path:
            import subprocess
            endpoint = json.loads(subprocess.check_output(['docker', 'context', 'inspect', '--format', '{{json .Endpoints.docker.Host}}'], text=True))
            if not endpoint.startswith('unix://'):
                raise RuntimeError('Resource monitor requires a local Docker Unix socket')
            path = endpoint.removeprefix('unix://')
        self.client = UnixHTTP(path)

    def snapshot(self):
        components = {}
        for name in self.containers:
            self.client.request('GET', f'/containers/{name}/stats?stream=false&one-shot=true')
            response = self.client.getresponse()
            data = json.loads(response.read())
            if response.status != 200:
                raise RuntimeError(f'{name}: Docker stats status {response.status}: {data}')
            memory = data['memory_stats']
            stat = memory.get('stats', {})
            inactive = stat.get('total_inactive_file', stat.get('inactive_file', 0))
            anonymous = stat.get('total_rss', stat.get('rss', stat.get('anon', 0)))
            shared = stat.get('total_shmem', stat.get('shmem', 0))
            components[name] = dict(cpu_total_ns=data['cpu_stats']['cpu_usage']['total_usage'],
                                    memory_usage_bytes=memory['usage'],
                                    working_set_bytes=max(0, memory['usage'] - inactive),
                                    anonymous_and_shared_bytes=anonymous + shared)
        combined = {key: sum(c[key] for c in components.values()) for key in next(iter(components.values()))}
        combined['components'] = components
        return combined

    def result(self):
        result = summarize(self.samples, ['working_set_bytes', 'anonymous_and_shared_bytes', 'memory_usage_bytes'], wall_seconds=self.wall)
        result['platform'] = 'Linux Docker cgroups'
        result['memory_accounting'] = 'working set = cgroup memory usage minus inactive file cache; anonymous+shared also provided; simultaneous app+DB sum, not sum of independent peaks'
        result['components'] = {}
        for name in self.containers:
            samples = [s['components'][name] | {'elapsed_seconds': s['elapsed_seconds']} for s in self.samples]
            result['components'][name] = summarize(samples, ['working_set_bytes', 'anonymous_and_shared_bytes', 'memory_usage_bytes'], wall_seconds=self.wall)
        return result


class DarwinUsage(ctypes.Structure):
    _fields_ = [('uuid', ctypes.c_uint8 * 16)] + [(name, ctypes.c_uint64) for name in (
        'user_time', 'system_time', 'pkg_idle_wkups', 'interrupt_wkups', 'pageins',
        'wired_size', 'resident_size', 'phys_footprint', 'proc_start_abstime',
        'proc_exit_abstime', 'child_user_time', 'child_system_time',
        'child_pkg_idle_wkups', 'child_interrupt_wkups', 'child_pageins',
        'child_elapsed_abstime', 'diskio_bytesread', 'diskio_byteswritten')]


class NativeMonitor(Monitor):
    def __init__(self, pid, output, interval=.25):
        super().__init__(output, interval)
        self.pid = pid
        self.before = resource.getrusage(resource.RUSAGE_CHILDREN)
        if platform.system() != 'Darwin':
            raise RuntimeError('Native memory sampler currently requires macOS')
        self.libproc = ctypes.CDLL('/usr/lib/libproc.dylib', use_errno=True)
        self.libproc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
        self.libproc.proc_pid_rusage.restype = ctypes.c_int
        class Timebase(ctypes.Structure):
            _fields_ = [('numer', ctypes.c_uint32), ('denom', ctypes.c_uint32)]
        timebase = Timebase()
        system = ctypes.CDLL('/usr/lib/libSystem.B.dylib')
        if system.mach_timebase_info(ctypes.byref(timebase)) != 0:
            raise RuntimeError('Unable to read Mach CPU clock scale')
        self.clock_numer, self.clock_denom = timebase.numer, timebase.denom

    def snapshot(self):
        usage = DarwinUsage()
        if self.libproc.proc_pid_rusage(self.pid, 2, ctypes.byref(usage)) != 0:
            return None  # Child exited between samples; final wait rusage is exact.
        # Darwin reports these CPU counters in Mach absolute ticks, not ns.
        cpu_ns = (usage.user_time + usage.system_time) * self.clock_numer // self.clock_denom
        return dict(cpu_total_ns=cpu_ns,
                    resident_bytes=usage.resident_size,
                    physical_footprint_bytes=usage.phys_footprint,
                    disk_read_bytes=usage.diskio_bytesread,
                    disk_written_bytes=usage.diskio_byteswritten)

    def result(self):
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        user = usage.ru_utime - self.before.ru_utime
        system = usage.ru_stime - self.before.ru_stime
        result = summarize(self.samples, ['resident_bytes', 'physical_footprint_bytes'], cpu_seconds=user + system, wall_seconds=self.wall)
        result.update(platform='native macOS', cpu_user_seconds=user,
                      cpu_system_seconds=system, kernel_peak_resident_bytes=usage.ru_maxrss,
                      memory_accounting='resident size and physical footprint from proc_pid_rusage v2; kernel lifetime peak RSS from wait rusage; SQLite is embedded in the same process')
        return result
