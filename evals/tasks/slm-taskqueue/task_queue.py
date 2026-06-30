import threading
import queue


class Future:
    def __init__(self):
        self._event = threading.Event()
        self._result = None
        self._error = None

    def set_result(self, result):
        self._result = result
        self._event.set()

    def set_error(self, error):
        self._error = error
        self._event.set()

    def result(self, timeout=None):
        self._event.wait(timeout=timeout)
        if self._error:
            raise self._error
        return self._result


class TaskQueue:
    def __init__(self, workers=4, maxsize=0):
        self._queue = queue.Queue(maxsize=maxsize)
        self._running = True
        self._workers = []
        for _ in range(workers):
            t = threading.Thread(target=self._worker, daemon=True)
            t.start()
            self._workers.append(t)

    def _worker(self):
        while True:
            try:
                item = self._queue.get(timeout=1.0)
            except queue.Empty:
                if not self._running:
                    break
                continue
            if item is None:
                break
            fn, args, kwargs, future = item
            try:
                result = fn(*args, **kwargs)
                future.set_result(result)
            except Exception as e:
                future.set_error(e)

    def submit(self, fn, *args, **kwargs):
        if not self._running:
            raise RuntimeError("Cannot submit new tasks after shutdown.")
        future = Future()
        self._queue.put((fn, args, kwargs, future))
        return future

    def shutdown(self, wait=True):
        self._running = False
        for _ in self._workers:
            self._queue.put(None)
        if wait:
            for worker in self._workers:
                worker.join()
