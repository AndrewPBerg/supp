#include "../include/circle.hpp"
#include <cmath>
#include <sstream>

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

Circle::Circle(double x, double y, double radius)
    : cx_(x), cy_(y), radius_(radius) {}

double Circle::area() const {
    return M_PI * radius_ * radius_;
}

double Circle::perimeter() const {
    return 2.0 * M_PI * radius_;
}

std::string Circle::describe() const {
    std::ostringstream ss;
    ss << "Circle(center=(" << cx_ << "," << cy_ << "), r=" << radius_ << ")";
    return ss.str();
}

double Circle::getRadius() const {
    return radius_;
}
