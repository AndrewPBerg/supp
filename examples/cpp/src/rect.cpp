#include "../include/rect.hpp"
#include <sstream>

Rect::Rect(double x, double y, double w, double h)
    : x_(x), y_(y), width_(w), height_(h) {}

double Rect::area() const {
    return width_ * height_;
}

double Rect::perimeter() const {
    return 2.0 * (width_ + height_);
}

std::string Rect::describe() const {
    std::ostringstream ss;
    ss << "Rect(origin=(" << x_ << "," << y_ << "), "
       << width_ << "x" << height_ << ")";
    return ss.str();
}

double Rect::getWidth() const {
    return width_;
}

double Rect::getHeight() const {
    return height_;
}
