package com.y;

import com.x.Cart;

public class Checkout {
    public long run(Cart cart) {
        return cart.total();
    }

    public long fq(com.x.Cart cart) {
        return cart.total();
    }

    public long price(com.x.Price price) {
        return price.amount();
    }
}
