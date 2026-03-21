#include <iostream>
#include <vector>
#include <memory>
#include "../include/circle.hpp"
#include "../include/rect.hpp"

void print_shape(const Shape& s) {
    std::cout << s.describe()
              << " area=" << s.area()
              << " perimeter=" << s.perimeter()
              << std::endl;
}

int main() {
    std::vector<std::unique_ptr<Shape>> shapes;
    shapes.push_back(std::make_unique<Circle>(0, 0, 5.0));
    shapes.push_back(std::make_unique<Rect>(1, 2, 10.0, 4.0));

    for (const auto& s : shapes) {
        print_shape(*s);
    }

    return 0;
}
