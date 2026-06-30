import time
import threading
from task_queue import TaskQueue
def test_concurrent():
    tq = TaskQueue(workers=3)
    
    def slow_task(n):
        time.sleep(0.1)
        return n * 2
    
    futures = []
    for i in range(30):
        futures.append(tq.submit(slow_task, i))
    
    results = sorted([f.result() for f in futures])
    expected = list(range(0, 60, 2))
    assert results == expected
    print("PASS: test_concurrent")
def test_exception():
    tq = TaskQueue(workers=1)
    
    def error_task():
        raise ValueError("Test exception handling")
    
    future = tq.submit(error_task)
    try:
        future.result()
        assert False, "Should have raised an exception"
    except ValueError as e:
        assert str(e) == "Test exception handling"
    print("PASS: test_exception")
def test_shutdown_wait():
    tq = TaskQueue(workers=1)
    
    def slow_task():
        time.sleep(0.5)
        return 42
    
    future = tq.submit(slow_task)
    tq.shutdown(wait=True)  # should join thread and complete the task
    assert future.result() == 42
    print("PASS: test_shutdown_wait")
def test_submit_after_shutdown():
    tq = TaskQueue(workers=1)
    tq.shutdown()
    try:
        tq.submit(lambda: None)
        assert False, "Should have raised an exception"
    except RuntimeError as e:
        assert str(e) == "Cannot submit new tasks after shutdown."
    print("PASS: test_submit_after_shutdown")
def test_backpressure():
    tq = TaskQueue(workers=1, maxsize=2)
    
    def slow_task():
        time.sleep(0.6)
        return None
    
    def fast_task():
        return None
    
    start = time.time()
    # Submit one slow task that will block the worker
    tq.submit(slow_task)
     # Queue is now full (1 executing + 2 waiting). The third .submit() call should block.
    tq.submit(fast_task)
    tq.submit(fast_task)
    start = time.time()
    future = tq.submit(fast_task)  # This one will wait for the queue to free up
    end = time.time()

    duration = end - start
    assert duration >= 0.4, f"Expected blocking time >= 0.4s but got {duration:.2f}s"
    future.result()
    print("PASS: test_backpressure")

if __name__ == "__main__":
    test_concurrent()
    test_exception()
    test_shutdown_wait()
    test_submit_after_shutdown()
    test_backpressure()