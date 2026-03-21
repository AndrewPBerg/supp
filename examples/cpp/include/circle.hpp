#ifndef CIRCLE_HPP
#define CIRCLE_HPP

#include "shape.hpp"

/** A circle defined by a center point and radius. */
class Circle : public Shape {
public:
    Circle(double x, double y, double radius);

    double area() const override;
    double perimeter() const override;
    std::string describe() const override;

    double getRadius() const;

private:
    double cx_, cy_, radius_;
};

#endif
