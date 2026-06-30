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
        tq = TaskQueue(workers=1, maxsize=2)  # Only allow 2 tasks in queue

        # Submit first two tasks
        tq.submit(test_task, 1)
        future = tq.submit(test_task, 2)

        # At this point the queue is full; third task should block
        start_time = time.time()

        def submit_third():
            tq.submit(test_task, 3)

        thread = threading.Thread(target=submit_third)
        thread.start()
        thread.join(timeout=0.5)  # Check if it's actually blocking

        # Now let the queue clear
        future.result()
        thread.join()

        end_time = time.time()
        self.assertGreaterEqual(end_time - start_time, 0.5)

if __name__ == "__main__":
    unittest.main()
