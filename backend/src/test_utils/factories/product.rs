//! Product factory for creating test products.
//!
//! Provides builders and factory functions for creating Product domain objects
//! with customizable attributes for testing purposes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Category of a product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ProductCategory {
    Electronics,
    Clothing,
    Books,
    Home,
    Sports,
    Toys,
    Food,
    #[default]
    Other,
}

/// Availability status of a product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Availability {
    #[default]
    InStock,
    LowStock,
    OutOfStock,
    Discontinued,
}

/// Product domain model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    /// Unique identifier for the product.
    pub id: Uuid,
    /// Product name.
    pub name: String,
    /// Product description.
    pub description: String,
    /// Price in cents.
    pub price_cents: i64,
    /// Product category.
    pub category: ProductCategory,
    /// SKU (Stock Keeping Unit).
    pub sku: String,
    /// Available quantity.
    pub quantity: u32,
    /// Availability status.
    pub availability: Availability,
    /// Weight in grams.
    pub weight_grams: Option<u32>,
    /// Image URLs.
    pub images: Vec<String>,
    /// Tags for search/filtering.
    pub tags: Vec<String>,
    /// Whether the product is featured.
    pub is_featured: bool,
    /// Timestamp when the product was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the product was updated.
    pub updated_at: DateTime<Utc>,
}

impl Default for Product {
    fn default() -> Self {
        Self::new()
    }
}

impl Product {
    /// Creates a new product with default values.
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: "Test Product".to_string(),
            description: "A test product description".to_string(),
            price_cents: 999,
            category: ProductCategory::default(),
            sku: format!("SKU-{}", Uuid::new_v4().to_string()[..8].to_uppercase()),
            quantity: 100,
            availability: Availability::default(),
            weight_grams: Some(500),
            images: Vec::new(),
            tags: Vec::new(),
            is_featured: false,
            created_at: now,
            updated_at: now,
        }
    }

    /// Creates a product with a specific name.
    pub fn with_name(name: impl Into<String>) -> Self {
        let mut product = Self::new();
        product.name = name.into();
        product
    }

    /// Creates an out-of-stock product.
    pub fn out_of_stock() -> Self {
        let mut product = Self::new();
        product.quantity = 0;
        product.availability = Availability::OutOfStock;
        product
    }

    /// Creates a featured product.
    pub fn featured() -> Self {
        let mut product = Self::new();
        product.is_featured = true;
        product
    }
}

/// Builder for creating Product instances.
#[derive(Debug, Clone)]
pub struct ProductFactory {
    product: Product,
}

impl Default for ProductFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ProductFactory {
    /// Creates a new ProductFactory with default values.
    pub fn new() -> Self {
        Self {
            product: Product::new(),
        }
    }

    /// Creates a ProductFactory from an existing Product.
    pub fn from_product(product: Product) -> Self {
        Self { product }
    }

    /// Sets the product's ID.
    pub fn id(mut self, id: Uuid) -> Self {
        self.product.id = id;
        self
    }

    /// Sets the product name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.product.name = name.into();
        self
    }

    /// Sets the product description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.product.description = description.into();
        self
    }

    /// Sets the price in cents.
    pub fn price_cents(mut self, price: i64) -> Self {
        self.product.price_cents = price;
        self
    }

    /// Sets the product category.
    pub fn category(mut self, category: ProductCategory) -> Self {
        self.product.category = category;
        self
    }

    /// Sets the SKU.
    pub fn sku(mut self, sku: impl Into<String>) -> Self {
        self.product.sku = sku.into();
        self
    }

    /// Sets the quantity.
    pub fn quantity(mut self, quantity: u32) -> Self {
        self.product.quantity = quantity;
        self.product.availability = if quantity == 0 {
            Availability::OutOfStock
        } else if quantity < 10 {
            Availability::LowStock
        } else {
            Availability::InStock
        };
        self
    }

    /// Sets the availability status.
    pub fn availability(mut self, availability: Availability) -> Self {
        self.product.availability = availability;
        self
    }

    /// Sets the weight in grams.
    pub fn weight_grams(mut self, weight: Option<u32>) -> Self {
        self.product.weight_grams = weight;
        self
    }

    /// Sets the image URLs.
    pub fn images(mut self, images: Vec<String>) -> Self {
        self.product.images = images;
        self
    }

    /// Adds an image URL.
    pub fn add_image(mut self, url: impl Into<String>) -> Self {
        self.product.images.push(url.into());
        self
    }

    /// Sets the tags.
    pub fn tags(mut self, tags: Vec<String>) -> Self {
        self.product.tags = tags;
        self
    }

    /// Adds a tag.
    pub fn add_tag(mut self, tag: impl Into<String>) -> Self {
        self.product.tags.push(tag.into());
        self
    }

    /// Sets the featured flag.
    pub fn is_featured(mut self, featured: bool) -> Self {
        self.product.is_featured = featured;
        self
    }

    /// Sets the created_at timestamp.
    pub fn created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.product.created_at = created_at;
        self
    }

    /// Sets the updated_at timestamp.
    pub fn updated_at(mut self, updated_at: DateTime<Utc>) -> Self {
        self.product.updated_at = updated_at;
        self
    }

    /// Builds and returns the Product instance.
    pub fn finish(self) -> Product {
        self.product
    }
}

/// Creates a Product with default values.
pub fn create() -> Product {
    Product::new()
}

/// Returns a new ProductFactory builder.
pub fn build() -> ProductFactory {
    ProductFactory::new()
}

/// Creates a Product with customizations applied via a closure.
pub fn create_with<F>(f: F) -> Product
where
    F: FnOnce(&mut Product),
{
    let mut product = Product::new();
    f(&mut product);
    product
}

/// Creates multiple Products with default values.
pub fn create_many(count: usize) -> Vec<Product> {
    (0..count).map(|_| Product::new()).collect()
}

/// Creates multiple Products with a builder function applied to each.
pub fn create_many_with<F>(count: usize, f: F) -> Vec<Product>
where
    F: Fn(&mut Product, usize),
{
    (0..count)
        .map(|i| {
            let mut product = Product::new();
            f(&mut product, i);
            product
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_product_with_defaults() {
        let product = create();
        assert!(!product.id.is_nil());
        assert_eq!(product.name, "Test Product");
        assert_eq!(product.price_cents, 999);
        assert_eq!(product.availability, Availability::InStock);
    }

    #[test]
    fn test_create_product_with_name() {
        let product = Product::with_name("Custom Product");
        assert_eq!(product.name, "Custom Product");
    }

    #[test]
    fn test_create_out_of_stock_product() {
        let product = Product::out_of_stock();
        assert_eq!(product.quantity, 0);
        assert_eq!(product.availability, Availability::OutOfStock);
    }

    #[test]
    fn test_create_featured_product() {
        let product = Product::featured();
        assert!(product.is_featured);
    }

    #[test]
    fn test_product_factory_basic() {
        let product = ProductFactory::new()
            .name("Factory Product")
            .price_cents(1999)
            .category(ProductCategory::Electronics)
            .finish();

        assert_eq!(product.name, "Factory Product");
        assert_eq!(product.price_cents, 1999);
        assert_eq!(product.category, ProductCategory::Electronics);
    }

    #[test]
    fn test_product_factory_quantity_updates_availability() {
        let product = ProductFactory::new().quantity(5).finish();

        assert_eq!(product.quantity, 5);
        assert_eq!(product.availability, Availability::LowStock);
    }

    #[test]
    fn test_product_factory_images() {
        let product = ProductFactory::new()
            .images(vec![
                "http://example.com/img1.jpg".to_string(),
                "http://example.com/img2.jpg".to_string(),
            ])
            .finish();

        assert_eq!(product.images.len(), 2);
    }

    #[test]
    fn test_product_factory_add_image() {
        let product = ProductFactory::new()
            .add_image("http://example.com/img1.jpg")
            .add_image("http://example.com/img2.jpg")
            .finish();

        assert_eq!(product.images.len(), 2);
    }

    #[test]
    fn test_product_factory_tags() {
        let product = ProductFactory::new()
            .tags(vec!["sale".to_string(), "new".to_string()])
            .finish();

        assert_eq!(product.tags.len(), 2);
    }

    #[test]
    fn test_product_factory_add_tag() {
        let product = ProductFactory::new()
            .add_tag("featured")
            .add_tag("sale")
            .finish();

        assert_eq!(product.tags, vec!["featured", "sale"]);
    }

    #[test]
    fn test_create_with_closure() {
        let product = create_with(|p| {
            p.name = "Closure Product".to_string();
            p.price_cents = 5000;
            p.category = ProductCategory::Books;
        });

        assert_eq!(product.name, "Closure Product");
        assert_eq!(product.price_cents, 5000);
        assert_eq!(product.category, ProductCategory::Books);
    }

    #[test]
    fn test_create_many() {
        let products = create_many(5);
        assert_eq!(products.len(), 5);
    }

    #[test]
    fn test_create_many_with() {
        let products = create_many_with(3, |p, i| {
            p.name = format!("Product {}", i);
            p.price_cents = 1000 * (i as i64 + 1);
        });

        assert_eq!(products.len(), 3);
        assert_eq!(products[0].name, "Product 0");
        assert_eq!(products[0].price_cents, 1000);
    }

    #[test]
    fn test_product_serialization() {
        let product = create();
        let json = serde_json::to_string(&product).unwrap();
        let parsed: Product = serde_json::from_str(&json).unwrap();

        assert_eq!(product.id, parsed.id);
        assert_eq!(product.name, parsed.name);
    }
}
