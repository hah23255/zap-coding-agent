import threading
import queue
import time
from typing import Any, Callable, Optional, List


class Future:
    def __init__(self):
        self._result = None
        self._exception = None
        self._done = False
        self._lock = threading.Lock()
        self._condition = threading.Condition(self._lock)

    def set_result(self, result: Any):
        with self._condition:
            self._result = result
            self._done = True
            self._condition.notify_all()

    def set_exception(self, exception: Exception):
        with self._condition:
            self._exception = exception
            self._done = True
            self._condition.notify_all()

    def result(self, timeout: Optional[float] = None) -> Any:
        with self._condition:
            if not self._done:
                self._condition.wait(timeout)
            if self._exception:
                raise self._exception
            return self._result


class TaskQueue:
    def __init__(self, workers: int = 4, maxsize: int = 0):
        self.maxsize = maxsize
        self.workers = workers
        self._queue: queue.Queue = queue.Queue(maxsize=maxsize)
        self._running = True
        self._lock = threading.Lock()

        self._workers: List[threading.Thread] = []
        for _ in range(workers):
            t = threading.Thread(target=self._worker)
            t.daemon = True
            t.start()
            self._workers.append(t)

    def submit(self, fn: Callable, *args, **kwargs) -> Future:
        future = Future()
        with self._lock:
            if not self._running:
                raise RuntimeError("Cannot submit tasks after shutdown")
        self._queue.put((fn, args, kwargs, future))
        return future

    def _worker(self):
        while True:
            try:
                item = self._queue.get(timeout=1.0)
            except queue.Empty:
                with self._lock:
                    if not self._running:
                        break
                continue
            if item is None:  # shutdown sentinel
                break
            fn, args, kwargs, future = item
            try:
                result = fn(*args, **kwargs)
                future.set_result(result)
            except Exception as e:
                future.set_exception(e)

    def shutdown(self, wait: bool = True):
        with self._lock:
            self._running = False
        for _ in self._workers:
            self._queue.put(None)
        if wait:
            for w in self._workers:
                w.join()