//! Order factory for creating test orders.
//!
//! Provides builders and factory functions for creating Order domain objects
//! with customizable attributes for testing purposes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of an order in the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum OrderStatus {
    /// Order has been created but not yet processed.
    #[default]
    Pending,
    /// Order is being processed.
    Processing,
    /// Order has been completed successfully.
    Completed,
    /// Order was cancelled.
    Cancelled,
    /// Order failed during processing.
    Failed,
}

/// Payment method for an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum PaymentMethod {
    #[default]
    CreditCard,
    DebitCard,
    PayPal,
    BankTransfer,
    Crypto,
}

/// Order item representing a product in an order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderItem {
    /// Unique identifier for this item.
    pub id: Uuid,
    /// Product ID.
    pub product_id: Uuid,
    /// Product name at time of purchase.
    pub product_name: String,
    /// Quantity ordered.
    pub quantity: u32,
    /// Unit price at time of purchase (in cents).
    pub unit_price_cents: i64,
}

impl OrderItem {
    /// Creates a new order item.
    pub fn new(
        product_id: Uuid,
        product_name: String,
        quantity: u32,
        unit_price_cents: i64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            product_id,
            product_name,
            quantity,
            unit_price_cents,
        }
    }
}

/// Order domain model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    /// Unique identifier for the order.
    pub id: Uuid,
    /// User ID who placed the order.
    pub user_id: Uuid,
    /// Status of the order.
    pub status: OrderStatus,
    /// Items in the order.
    pub items: Vec<OrderItem>,
    /// Total amount in cents.
    pub total_cents: i64,
    /// Currency code (e.g., "USD").
    pub currency: String,
    /// Payment method used.
    pub payment_method: PaymentMethod,
    /// Shipping address.
    pub shipping_address: String,
    /// Billing address.
    pub billing_address: String,
    /// Timestamp when the order was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the order was updated.
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the order was completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// Notes or special instructions.
    pub notes: Option<String>,
}

impl Default for Order {
    fn default() -> Self {
        Self::new()
    }
}

impl Order {
    /// Creates a new order with default values.
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            status: OrderStatus::default(),
            items: Vec::new(),
            total_cents: 0,
            currency: "USD".to_string(),
            payment_method: PaymentMethod::default(),
            shipping_address: "123 Test St, Test City, TC 12345".to_string(),
            billing_address: "123 Test St, Test City, TC 12345".to_string(),
            created_at: now,
            updated_at: now,
            completed_at: None,
            notes: None,
        }
    }

    /// Creates a new order for a specific user.
    pub fn for_user(user_id: Uuid) -> Self {
        let mut order = Self::new();
        order.user_id = user_id;
        order
    }

    /// Adds an item to the order.
    pub fn add_item(&mut self, item: OrderItem) {
        self.total_cents += item.unit_price_cents * item.quantity as i64;
        self.items.push(item);
    }

    /// Calculates the total from items.
    pub fn recalculate_total(&mut self) {
        self.total_cents = self
            .items
            .iter()
            .map(|item| item.unit_price_cents * item.quantity as i64)
            .sum();
    }
}

/// Builder for creating Order instances.
#[derive(Debug, Clone)]
pub struct OrderFactory {
    order: Order,
}

impl Default for OrderFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl OrderFactory {
    /// Creates a new OrderFactory with default values.
    pub fn new() -> Self {
        Self {
            order: Order::new(),
        }
    }

    /// Creates an OrderFactory from an existing Order.
    pub fn from_order(order: Order) -> Self {
        Self { order }
    }

    /// Sets the order's ID.
    pub fn id(mut self, id: Uuid) -> Self {
        self.order.id = id;
        self
    }

    /// Sets the user ID.
    pub fn user_id(mut self, user_id: Uuid) -> Self {
        self.order.user_id = user_id;
        self
    }

    /// Sets the order status.
    pub fn status(mut self, status: OrderStatus) -> Self {
        self.order.status = status;
        self
    }

    /// Sets the order items.
    pub fn items(mut self, items: Vec<OrderItem>) -> Self {
        self.order.items = items;
        self.order.recalculate_total();
        self
    }

    /// Adds a single item to the order.
    pub fn add_item(mut self, item: OrderItem) -> Self {
        self.order.add_item(item);
        self
    }

    /// Sets the total amount in cents.
    pub fn total_cents(mut self, total: i64) -> Self {
        self.order.total_cents = total;
        self
    }

    /// Sets the currency.
    pub fn currency(mut self, currency: impl Into<String>) -> Self {
        self.order.currency = currency.into();
        self
    }

    /// Sets the payment method.
    pub fn payment_method(mut self, method: PaymentMethod) -> Self {
        self.order.payment_method = method;
        self
    }

    /// Sets the shipping address.
    pub fn shipping_address(mut self, address: impl Into<String>) -> Self {
        self.order.shipping_address = address.into();
        self
    }

    /// Sets the billing address.
    pub fn billing_address(mut self, address: impl Into<String>) -> Self {
        self.order.billing_address = address.into();
        self
    }

    /// Sets the created_at timestamp.
    pub fn created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.order.created_at = created_at;
        self
    }

    /// Sets the updated_at timestamp.
    pub fn updated_at(mut self, updated_at: DateTime<Utc>) -> Self {
        self.order.updated_at = updated_at;
        self
    }

    /// Sets the completed_at timestamp.
    pub fn completed_at(mut self, completed_at: Option<DateTime<Utc>>) -> Self {
        self.order.completed_at = completed_at;
        self
    }

    /// Sets the notes.
    pub fn notes(mut self, notes: Option<String>) -> Self {
        self.order.notes = notes;
        self
    }

    /// Builds and returns the Order instance.
    pub fn finish(self) -> Order {
        self.order
    }
}

/// Creates an Order with default values.
pub fn create() -> Order {
    Order::new()
}

/// Returns a new OrderFactory builder.
pub fn build() -> OrderFactory {
    OrderFactory::new()
}

/// Creates an Order for a specific user.
pub fn create_for_user(user_id: Uuid) -> Order {
    Order::for_user(user_id)
}

/// Creates an Order with customizations applied via a closure.
pub fn create_with<F>(f: F) -> Order
where
    F: FnOnce(&mut Order),
{
    let mut order = Order::new();
    f(&mut order);
    order
}

/// Creates multiple Orders with default values.
pub fn create_many(count: usize) -> Vec<Order> {
    (0..count).map(|_| Order::new()).collect()
}

/// Creates multiple Orders with a builder function applied to each.
pub fn create_many_with<F>(count: usize, f: F) -> Vec<Order>
where
    F: Fn(&mut Order, usize),
{
    (0..count)
        .map(|i| {
            let mut order = Order::new();
            f(&mut order, i);
            order
        })
        .collect()
}

/// Creates an order with items.
pub fn create_with_items(items: Vec<OrderItem>) -> Order {
    let mut order = Order::new();
    for item in items {
        order.add_item(item);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_order_item() -> OrderItem {
        OrderItem::new(Uuid::new_v4(), "Test Product".to_string(), 2, 1999)
    }

    #[test]
    fn test_create_order_with_defaults() {
        let order = create();
        assert!(!order.id.is_nil());
        assert_eq!(order.status, OrderStatus::Pending);
        assert_eq!(order.currency, "USD");
        assert!(order.items.is_empty());
    }

    #[test]
    fn test_create_order_for_user() {
        let user_id = Uuid::new_v4();
        let order = create_for_user(user_id);
        assert_eq!(order.user_id, user_id);
    }

    #[test]
    fn test_order_factory_basic() {
        let order = OrderFactory::new()
            .status(OrderStatus::Completed)
            .total_cents(5000)
            .finish();

        assert_eq!(order.status, OrderStatus::Completed);
        assert_eq!(order.total_cents, 5000);
    }

    #[test]
    fn test_order_factory_with_items() {
        let items = vec![
            OrderItem::new(Uuid::new_v4(), "Product 1".to_string(), 1, 1000),
            OrderItem::new(Uuid::new_v4(), "Product 2".to_string(), 2, 500),
        ];

        let order = OrderFactory::new().items(items).finish();

        assert_eq!(order.items.len(), 2);
        assert_eq!(order.total_cents, 2000); // 1*1000 + 2*500
    }

    #[test]
    fn test_order_add_item() {
        let mut order = Order::new();
        order.add_item(sample_order_item());

        assert_eq!(order.items.len(), 1);
        assert_eq!(order.total_cents, 3998); // 2 * 1999
    }

    #[test]
    fn test_order_recalculate_total() {
        let mut order = Order::new();
        order.items = vec![
            OrderItem::new(Uuid::new_v4(), "Product 1".to_string(), 3, 100),
            OrderItem::new(Uuid::new_v4(), "Product 2".to_string(), 2, 200),
        ];
        order.recalculate_total();

        assert_eq!(order.total_cents, 700); // 3*100 + 2*200
    }

    #[test]
    fn test_create_with_closure() {
        let order = create_with(|o| {
            o.status = OrderStatus::Cancelled;
            o.notes = Some("Cancelled by user".to_string());
        });

        assert_eq!(order.status, OrderStatus::Cancelled);
        assert_eq!(order.notes, Some("Cancelled by user".to_string()));
    }

    #[test]
    fn test_create_many() {
        let orders = create_many(3);
        assert_eq!(orders.len(), 3);
    }

    #[test]
    fn test_order_serialization() {
        let order = create();
        let json = serde_json::to_string(&order).unwrap();
        let parsed: Order = serde_json::from_str(&json).unwrap();

        assert_eq!(order.id, parsed.id);
        assert_eq!(order.total_cents, parsed.total_cents);
    }
}
