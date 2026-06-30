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


class ThreadSafeQueue:
    def __init__(self, maxsize: int = 0):
        self._queue = queue.Queue(maxsize=maxsize)
        self._lock = threading.Lock()

    def qsize(self) -> int:
        with self._lock:
            return self._queue.qsize()

    def put(self, item: Any, block: bool = True, timeout: Optional[float] = None):
        with self._lock:
            self._queue.put(item, block=block, timeout=timeout)

    def get(self, block: bool = True, timeout: Optional[float] = None) -> Any:
        with self._lock:
            return self._queue.get(block=block, timeout=timeout)


class TaskQueue:
    def __init__(self, workers: int = 4, maxsize: int = 0):
        self.maxsize = maxsize
        self.workers = workers
        self._queue = ThreadSafeQueue(maxsize)
        self._futures = {}
        self._running = True
        self._lock = threading.Lock()
        self._shutdown_complete = threading.Event()

        # Start worker threads
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

        # Add task to queue
        self._queue.put((fn, args, kwargs, future))
        return future

    def _worker(self):
        try:
            while self._running or not self._queue.qsize() == 0:
                try:
                    fn, args, kwargs, future = self._queue.get(timeout=1.0)
                    try:
                        result = fn(*args, **kwargs)
                        future.set_result(result)
                    except Exception as e:
                        future.set_exception(e)
                except queue.Empty:
                    if not self._running:
                        break
        finally:
            pass

    def shutdown(self, wait: bool = True):
        with self._lock:
            self._running = False

        # Signal workers to exit
        for _ in range(self.workers):
            self._queue.put(None)

        # Wait for all tasks to complete
        if wait:
            for w in self._workers:
                w.join()