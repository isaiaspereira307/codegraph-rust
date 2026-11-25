package com.x;

public class OrderService {
    public long run(Cart cart) {
        return cart.total();
    }
}
