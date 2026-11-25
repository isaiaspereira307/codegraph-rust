package com.y;

import org.springframework.beans.factory.annotation.Autowired;

public class Billing {
    @Autowired
    private Payment payment;

    public void run() {
        payment.charge();
    }
}
