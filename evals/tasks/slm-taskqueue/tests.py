import threading
import time
import unittest
from concurrent.futures import ThreadPoolExecutor

from task_queue import TaskQueue

def test_task(x, wait_time=0):
    time.sleep(wait_time)
    return x * 2

class TestTaskQueue(unittest.TestCase):

    def test_concurrent_tasks(self):
        tq = TaskQueue(workers=4, maxsize=10)

        def submit_tasks():
            results = []
            for i in range(10):
                future = tq.submit(test_task, i)
                results.append(future.result())
            return results

        with ThreadPoolExecutor(max_workers=3) as executor:
            futures = [executor.submit(submit_tasks) for _ in range(3)]
            all_results = []
            for f in futures:
                all_results.extend(f.result())

        expected = [(i * 2) for i in range(10)] * 3
        self.assertEqual(sorted(all_results), sorted(expected))

    def test_exception_propagation(self):
        tq = TaskQueue()

        def task_throws():
            raise ValueError("Test exception")

        future = tq.submit(task_throws)
        with self.assertRaises(ValueError) as ctx:
            future.result()
        self.assertEqual(str(ctx.exception), "Test exception")

    def test_shutdown_wait(self):
        tq = TaskQueue(workers=2, maxsize=5)

        start_time = time.time()

        # Submit tasks that take some time
        futures = [tq.submit(test_task, i, wait_time=1) for i in range(10)]

        # Shutdown and wait
        tq.shutdown(wait=True)

        end_time = time.time()
        # Check if shutdown waited for all in-flight tasks to complete (~2 seconds total)
        self.assertGreaterEqual(end_time - start_time, 2.0)

    def test_submit_after_shutdown(self):
        tq = TaskQueue()

        tq.shutdown(wait=False)
        with self.assertRaises(RuntimeError):
            tq.submit(test_task, 5)

    def test_maxsize_backpressure(self):
        # 1 worker, maxsize=2. Pin the worker with a slow task so the queue
        # stays full and the 4th submit actually blocks.
        tq = TaskQueue(workers=1, maxsize=2)

        tq.submit(test_task, 1, wait_time=0.6)  # occupies the single worker
        tq.submit(test_task, 2)                  # queued (1/2)
        tq.submit(test_task, 3)                  # queued (2/2) — full

        start_time = time.time()

        def submit_fourth():
            tq.submit(test_task, 4)  # must block until a slot opens

        thread = threading.Thread(target=submit_fourth)
        thread.start()
        thread.join(timeout=0.5)  # should still be blocking after 0.5 s
        self.assertTrue(thread.is_alive(), "submit() should have blocked")

        thread.join()  # let it complete naturally
        end_time = time.time()
        tq.shutdown(wait=True)
        self.assertGreaterEqual(end_time - start_time, 0.5)

if __name__ == "__main__":
    unittest.main()
