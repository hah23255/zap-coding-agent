class Node:
    def __init__(self, key: int, value: int):
        self.key = key
        self.value = value
        self.prev = None
        self.next = None

class LRUCache:
    def __init__(self, capacity: int):
        self.capacity = capacity
        self.cache = {}
        self.head = Node(0, 0)
        self.tail = Node(0, 0)
        self.head.next = self.tail
        self.tail.prev = self.head

    def _remove(self, node):
        prev_node = node.prev
        next_node = node.next
        prev_node.next = next_node
        next_node.prev = prev_node

    def _add_to_head(self, node):
        node.prev = self.head
        node.next = self.head.next
        self.head.next.prev = node
        self.head.next = node

    def get(self, key):
        if key in self.cache:
            node = self.cache[key]
            self._remove(node)
            self._add_to_head(node)
            return node.value
        return -1

    def put(self, key, value):
        if key in self.cache:
            node = self.cache[key]
            node.value = value
            self._remove(node)
            self._add_to_head(node)
        else:
            if len(self.cache) >= self.capacity:
                lru_node = self.tail.prev
                self._remove(lru_node)
                del self.cache[lru_node.key]
            new_node = Node(key, value)
            self._add_to_head(new_node)
            self.cache[key] = new_node

def run_tests():
    print("Running LRU Cache tests...")
    cache = LRUCache(2)
    cache.put(1, 1)
    assert cache.get(1) == 1
    print("Basic get/put: Passed")

    cache.put(2, 2)
    cache.put(3, 3)
    assert cache.get(2) == 2
    assert cache.get(1) == -1
    print("Capacity eviction: Passed")

    cache.put(4, 4)
    cache.get(2)
    cache.put(5, 5)
    assert cache.get(2) == 2
    assert cache.get(1) == -1
    print("Recency update on get: Passed")

    cache.put(2, 2)
    cache.put(6, 6)
    assert cache.get(2) == 2
    print("Recency update on put: Passed")

    cache = LRUCache(1)
    cache.put(2, 2)
    cache.put(3, 3)
    assert cache.get(2) == -1
    print("Single-capacity edge case: Passed")

    cache = LRUCache(2)
    cache.put(2, 2)
    cache.put(3, 3)
    cache.put(3, 6)
    assert cache.get(3) == 6
    print("Overwrite existing key: Passed")

    print("All tests passed!")

run_tests()
